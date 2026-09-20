# 安全策略

[English](https://github.com/z2net/bffi-rs/blob/main/SECURITY.md) | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/SECURITY.md) | **[简体中文](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/SECURITY.md)**

## 支持的版本

| 版本 | 支持情况 |
| --- | --- |
| 0.1.x | 针对最新的 `0.1.x` 发布提供安全修复 |

`0.0.x` 发布以及更早的 `0.1.x` 小版本不再获得修复 - 请升级到最新的补丁版本。

## 报告漏洞

- 首选方式:GitHub **私有漏洞报告**
  (Security 标签页 -> Report a vulnerability)。
- 邮箱:**contact@z2net.com**

请包含:问题描述、复现步骤、潜在影响、受影响的版本。

我们的目标是在 72 小时内确认收到(评为 Critical 的报告为 24 小时)。修复以协同披露的方式发布 - 在修复发布之前,请勿公开披露。如需要,会注明报告者署名。

## 范围

- FFI 边界中的内存安全缺陷
- 句柄表损坏 / 类型混淆
- 跨模块的句柄混淆(注册表身份)
- panic 传播问题 / `catch_unwind` 边界策略
- 生成的包装函数(`bffi-macros`)中可能隐藏 `unsafe` 的代码生成缺陷
- **加载器清单完整性**:`.bffi/bffi.api.json` 与已构建 cdylib 之间的篡改(绕过 ABI 握手)、平台包完整性校验绕过
- 工件解析中的**路径穿越**(`libraryPath`、`crate.dir`、平台包解析)
- 畸形的 wire 负载:整数溢出、通过声明长度进行的分配 DoS、无界嵌套
- 竞态条件、use-after-free、回调死锁(包括重入的 `invoke_wait`)
- 发布流水线的供应链(release-npm.yml / release-native.yml):工件替换、来源溯源缺口

不在范围内:
- Bun 本身的缺陷(请向 oven-sh/bun 报告)
- Rust 工具链的缺陷
- 加载后行为恶意的原生模块:加载 cdylib 本质上就是任意代码执行 - 加载器握手防范的是意外的版本陈旧和被篡改的清单,而不是你自己选择加载的恶意二进制

## 信任模型(简版)

- `.bffi/` 配置与清单是**受信任的输入**:绝不提交或加载你没有审查过的 `.bffi/bffi.json`。`libraryPath` 绕过平台解析,是一项显式的信任决定(`trust: "explicit"`)。
- bffi 提供 **panic 遏制**(release 包装函数尽可能把 Rust panic 转换为 JS 错误),而非**进程隔离**:原生模块运行在 Bun 进程之内。原生代码中的内存损坏、`abort`、段错误或分配器破坏随时可以击垮宿主。
- `e.nativeStack` 受 `RUST_BACKTRACE` 门控 - 生产环境请保持关闭(它会泄露路径与代码结构)。
- 发布配置中的 `panic = "unwind"` 是必需的:模块 `[profile.release]` 中的 `panic = "abort"` 会破坏遏制策略并中止宿主(`bffi doctor` 会检查这一点)。

## 运行时加固

- 执行不受信任 JavaScript 的宿主可以在启动 Bun 时附加 `--no-ffi-cc`(或 `--no-addons`),以禁止通过 `bun:ffi` 的 `cc()` 在运行时编译和加载 C 代码(该标志自 Bun 1.4.1 起可用)。bffi 自身从不调用 `cc()`。
