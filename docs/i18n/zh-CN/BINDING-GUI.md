# 从 Bun 绑定 GUI 与事件驱动库

[English](https://github.com/z2net/bffi-rs/blob/main/docs/BINDING-GUI.md) | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/BINDING-GUI.md) | **简体中文**

bffi 是一个绑定层(Bun 生态中的 napi-rs 等价物):原生 Rust 库编译为 cdylib,Bun 通过类型化加载器加载并调用它。本指南涵盖普通 napi-rs 做不好的那一个场景:**GUI / 事件驱动库**(wry、winit、tao、SDL 等),它们的处理器运行在操作系统线程上,需要与 JavaScript 进行同步往返。

可用的参考实现是 [`wry`](https://github.com/z2net/bffi-examples/tree/main/wry)(一个由 Bun 驱动的完整 webview 窗口)。请将它与本指南对照阅读。

## 线程模型

GUI 库拥有一个操作系统级的事件循环,而该循环必须运行在专用线程上。模式如下:

```text
Bun main thread (JS)                native loop thread (cdylib)
----------------------              ---------------------------
api.webview_open(cfg)  --proxy-->   EventLoop + Window + WebView
api.webview_eval(h, js) --proxy-->   evaluate_script
      |                                    |
      | pump the bffi event loop           | ipc_handler fires
      |<-- marshalled callback job ------- invoke_wait(js_cb, ...)
      |      (runs on THIS thread)         | (parked until reply)
      | resolve via webview_eval ---->     | wakes with the reply
```

- cdylib 在首次调用时派生循环线程(`std::thread` 加一条 `EventLoopProxy` 命令通道)。JavaScript 永远不会因窗口而阻塞:每个原生入口点都只是代理一条命令并立即返回。
- **JS 线程保持空闲,以便泵送** bffi 事件循环(`pumpUntil` / 生成的 `loop_pump`,即 `docs/DESIGN.md` 中的显式泵送契约)。投递只在 JS 线程泵送期间执行 - 框架从不安装隐藏的定时器。
- 跨线程合法性:在 Windows 上,winit/tao 循环在主线程之外运行毫无问题(`with_any_thread(true)`)。Linux 工具包(webkit2gtk/gtk-init)的假设更严格 - 在那里要么给平台加门控,要么把循环放到主线程上泵送。

## 从原生线程调用 JavaScript:invoke_wait

让这一切成立的核心原语只有一个:

```rust
use bffi::{invoke_wait, Value};
use std::time::Duration;

// Runs on the GUI loop thread: calls the JS-bound callback,
// parks until the JS side answers (it pumps), returns the value.
let reply = invoke_wait(ipc_handle, &[Value::Str(body)],
                        Duration::from_secs(30))?;
```

- `invoke_wait` 同时分派两张回调表:原生注册的闭包(`bffi::register`,`Value` 路径)与 JS 绑定的回调(`bffi_callback_bind` - JavaScript 绑定自己的函数时拿到的句柄)。
- 调用被入队到 bffi 事件循环;发起调用的线程在一个槽位上等待;当 JS 线程泵送时,任务就在那里执行(JSCallback 与线程绑定 - 这是唯一合法的方式),结果再经槽位传回。
- **超时是强制的。** 从不被泵送的循环不得挂住原生线程:等待是有界的,会返回 `ErrorCode::Timeout`(15),而迟到的结果会无害地落在被废弃的槽位里。
- **死锁契约**(记录在 `crates/bffi/CALLING-CONVENTION.md` 第 9.1 节):JS 线程必须泵送;绝不要从本身就运行在 JS 线程泵送回调内的代码里调用 `invoke_wait`(重入)- 这种调用按设计以超时告终。
- JS 绑定分派的 C 调用矩阵:至多 2 个 `i32 | i64 | u64 | f64 | bool | cstring` 参数,返回 `i32 | i64 | u64 | f64 | bool | cstring | void`(cstring 返回值立即拷出;指针仅在调用期间有效)。二进制负载与组合类型(数组、record)无法穿越原始 C 调用 - 它们改走缓冲通道;不支持的形状会快速失败(`UnsupportedSignature`),不会造成阻塞。

## 配置结构体:record 中的 Option 字段

GUI 构造器接受稀疏的配置。`#[derive(BffiRecord)]` 结构体在整个字段矩阵上都接受 `Option<T>` 字段 - `None` 以 wire 的 `TAG_UNIT` 字节跨过边界,`Some(v)` 则作为普通值;TypeScript 将该字段渲染为 `T | null`:

```rust
#[derive(bffi::BffiRecord)]
pub struct WebviewConfig {
    pub url: Option<String>,
    pub html: Option<String>,
    pub title: Option<String>,
    pub width: Option<u32>,
    pub devtools: Option<bool>,
}
```

嵌套的 `Option<Option<T>>` 会在编译期被拒绝(E010)。

## IPC 往返

完整的 UI -> 原生 -> JS -> 原生 -> UI 循环(参见 [`wry/src/lib.rs`](https://github.com/z2net/bffi-examples/blob/main/wry/src/lib.rs)):

1. JavaScript 绑定自己的处理器(`bffi_callback_bind`,签名为 `unit(str)`),并把句柄交给一个原生导出(`webview_bind_ipc`)。
2. 页面投递一条消息(`window.ipc.postMessage`);库的 `ipc_handler` 在循环线程上运行,调用 `invoke_wait(ipc_handle, &[Value::Str(body)], 30s)`。
3. JS 线程泵送,被绑定的处理器得以运行,其应答经 `webview_eval` 送回(解析页面里的 promise,例如 `window.__bffiResolve(id, json)`)。
4. 循环线程带着结果醒来并返回。

处理器要保持快速,也绝不要让多个往返相互嵌套(每页同时只保留一个进行中的调用,协议就能保持极简)。

## 保活与关闭

- **回调在 JS 侧保持存活**:被绑定的 JSCallback 及其句柄必须活得与原生侧能调用它们的时长一致(把对象保存在模块级注册表中;关闭时显式撤销)。原生侧在分派时会重新检查句柄 - 已撤销的句柄表现为 `InvalidHandle`,而不是崩溃。
- **关闭协议**:丢弃最后一个窗口会让循环线程退出(`EventLoop::exit`);发布一个粘性的已退出标志,让 JavaScript 去观察它(示例中的 `webview_poll_exit`)。Bun 退出时,循环线程上注册的 `Drop` 守卫会运行 - 不要为自行派生的线程依赖 atexit。

## 打包说明

- **Windows**:WebView2 加载器(`WebView2Loader.dll`)必须能在宿主二进制旁边找到 - 随平台包一起发布;WebView2 *运行时*本身是操作系统组件(`bffi doctor` 可以探测它)。
- **Linux**:webkit2gtk 及其 GTK 依赖是动态链接的(构建需安装 `libwebkit2gtk-4.1-dev` / `libgtk-3-dev`);headless CI 做真实窗口测试需要 `xvfb`。
- **macOS**:WKWebView 是操作系统的一部分;AppKit 的窗口必须在主线程上创建 - 本指南的主线程外模式目前仅适用于 Windows。
- 二进制就是普通的 cdylib:同样的 `bffi pack` / 平台包流程原样适用。

## 新绑定的核对清单

1. 锁定库版本(`=x.y.z`) - GUI crate 迭代很快。
2. 带 `Option` 字段的配置结构体;record 表达不了的地方用 builder setter。
3. 循环线程 + 代理命令通道;每个导出立即返回。
4. 库的每个处理器都配 JS 绑定回调;`invoke_wait` 带有界超时;原生 -> UI 用 `evaluate` 式导出。
5. 粘性退出标志 + 轮询导出;显式的 `close` 导出。
6. 无窗口的 Rust 测试;真实窗口 e2e 放在环境变量门控之后(示例中的 `BFFI_WRY_E2E=1`)。
7. README 附线程图和启动命令。
