# AGENTS.md - AI 代理与贡献者规则

[English](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/AGENTS.md) | **简体中文**

本文件定义了人类与 AI 代理在 **bffi-rs** 上工作时必须遵循的规则。

仓库:https://github.com/z2net/bffi-rs  
联系方式:contact@z2net.com

---

## 1. 项目目的

`bffi-rs` 是一个用 Rust 编写的**仅支持 Bun** 的原生绑定框架。

它是 `napi-rs` 在 Bun 上的等价物,但:

- 仅面向 **Bun**(无 Node.js / Deno 兼容性);
- **不**依赖 Node-API;
- 使用 `bun:ffi` 和一层薄的 C ABI 层;
- 以**自底向上的模块栈**组织(基础模块在前,门面在后),置于一个小型的三 crate 工作区中。

首要目标:FFI 边界的安全性、清晰的所有权、良好的 DX 以及长期可维护性。

在进行架构更改之前,请先阅读 `docs/DESIGN.md`。

---

## 2. 硬性规则

1. **仅支持 Bun**
   不要添加 Node.js 或 Deno 兼容层。

2. **安全第一**
   - 默认路径 = 复制数据,绝不零拷贝。
   - 仅允许通过 `bffi::unsafe_zero_copy` 进行零拷贝。
   - 所有 `extern "C"` 函数必须保持精简,并用 `catch_unwind` 包裹。

3. **句柄**
   使用 Generational Index + type-tag(`u64`)。
   绝不通过 C ABI 暴露原始 Rust 引用或复杂类型。

4. **Panic**
   - 开发构建可以中止(更易于调试)。
   - 生产构建必须将 panic 转换为 JS `Error`。

5. **最低 Bun 版本**
   `1.4.2`

6. **Rust / Cargo 版本**
   项目锁定为 **Cargo / Rust 1.98.0**。
   未经明确决策和 CI 更新,不得提升版本。

7. **仓库中不得包含机密信息**
   任何位于 `.grok`、`.claude`、`.codex`、`.opencode`、`.zcode`、`.hermes`、`.mcp`、`.mimosa`、`.env` 下的内容,以及密钥、令牌等,都必须排除在 git 之外(参见 `.gitignore`)。

8. **许可证**
   MIT。在适当之处保留 SPDX 头部声明。

---

## 3. 仓库结构

```
bffi-rs/
├── AGENTS.md                    # this file
├── README.md
├── LICENSE
├── SECURITY.md
├── CONTACT.md
├── CHANGELOG.md
├── Cargo.toml                  # workspace (3 members)
├── deny.toml                   # cargo-deny / cargo-audit policy (CI supply-chain job)
├── rust-toolchain.toml         # pinned 1.98.0
├── package.json                # Bun workspace / scripts
├── bun.lock
├── tsconfig.json
├── .oxlintrc.json              # linter config
├── lefthook.yml                # git hooks (lint, fmt, commit-msg)
├── .gitattributes              # golden-file diff policy
├── .gitignore
├── .github/
│   ├── ISSUE_TEMPLATE/
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── workflows/              # ci, fuzz, bench, release-native, release-crates, release-npm
├── crates/
│   ├── bffi/                    # the runtime stack as layered modules (core, types, error,
│   │                            #   object, callback, dts, build, event_loop, async, stream)
│   │                            #   + the public facade; CALLING-CONVENTION.md lives here
│   ├── bffi-macros/             # all proc-macros: #[bffi], #[bffi_async], #[bffi_stream],
│   │                            #   #[bffi_class]/#[bffi_impl], derives (BffiRecord/BffiEnum/
│   │                            #   BffiError); src/support/ = shared macro internals,
│   │                            #   src/class/ = the class family
│   └── bffi-native/             # reference cdylib (runtime ABI; -> @z2net/bffi-native packages)
├── fuzz/                        # self-contained cargo-fuzz workspace (nightly; fuzz.yml)
├── docs/
│   ├── DESIGN.md                # architecture & decisions
│   ├── BINDING-GUI.md           # GUI / event-driven libraries guide
│   ├── CONTRIBUTING.md
│   ├── CODE_OF_CONDUCT.md
│   └── i18n/                    # ru / zh-CN translations (README, DESIGN, AGENTS, ...)
├── packages/                      # JS-side: bffi (@z2net/bffi), bffi-cli, native
└── scripts/                      # commit-msg hook + bench driver

示例位于独立仓库:https://github.com/z2net/bffi-examples
(每个示例都是独立的 crate,同时也是一个针对已发布包的 e2e 测试套件)。
```

运行时栈保持为小型、单一职责的模块(`bffi_core`、`bffi_types`、
`bffi_error`、`bffi_object`、`bffi_callback`、`bffi_dts`、`bffi_build`、
`bffi_event_loop`、`bffi_async`、`bffi_stream`),位于 `crates/bffi/src/`
内,自底向上分层,顶层是 `bffi` 门面。新 crate 必须遵循 `bffi-*` 命名
方案并加入 workspace;新模块必须保持自底向上的分层及其 feature 门控。

---

## 4. 开发工作流

### 环境配置

```bash
# Rust
rustup toolchain install 1.98.0
rustup default 1.98.0

# Bun
bun install
```

### 常用命令

```bash
bun run lint          # oxlint
bun run typecheck     # tsc
bun run build         # 构建参考 cdylib(release)
bun run test:js       # 运行包的单元测试(bun test packages)
bun run ci            # full CI parity: lint, typecheck, fmt, clippy, tests, JS tests
cargo check
cargo test
cargo fmt
cargo clippy
```

### 提交风格

我们使用 **Conventional Commits**:

```
feat: add generational handle table
fix: prevent panic across FFI boundary
docs: update DESIGN.md decisions
refactor(core): simplify catch_unwind helper
test: cover buffer copy path
chore: pin rust-toolchain to 1.98.0
```

破坏性变更必须在页脚使用 `BREAKING CHANGE:`,或在类型之后加上 `!`。

### 分支与发布

- `main` - 生产分支;进入 `main` 的 PR 仅能由项目所有者从 `dev/main` 创建。
- `dev/main` - 集成分支;所有功能工作都通过 PR 汇入此处。
- 功能在 `dev/<feature>` 分支(kebab-case)中开发,从 `dev/main` 切出并合并回 `dev/main`。
- PR `dev/<feature>` → `dev/main` 需要 1 个批准以及绿色的 CI(`.github/workflows/ci.yml`;本地为 `bun run ci`)。
- 发布标签 `v<semver>`(附注标签)仅放置在 `main` 上,且仅由所有者创建。

完整规则:[docs/CONTRIBUTING.md](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/CONTRIBUTING.md) → “分支与发布”。

### 拉取请求

- 每个 PR 只包含一个逻辑变更。
- CI 必须通过。
- 当行为或公共 API 发生变化时,更新文档。
- 引用相关的 issue。

---

## 5. AI 代理规则

在此仓库上工作时,AI 代理**必须**:

1. 在进行大规模更改之前,先阅读 `DESIGN.md` 和本文件。
2. 优先采用小的、易于审查的 diff。
3. 绝不提交机密信息、个人 AI 配置或 `.env` 文件。
4. 不引入 Node/Deno 兼容性。
5. 保持分层架构:基础模块在前,`bffi` 门面在后;不得绕过 `crates/bffi` 内部的 `bffi_*` 模块边界。
6. 保持安全模型(默认复制、显式的 unsafe 零拷贝、代际句柄)。
7. 在可能的情况下运行 `cargo fmt`、`cargo clippy` 和测试。
8. 如果某项决策发生变化,更新 `DESIGN.md` 或相关文档。

对架构拿不准时,优先选择询问(或打开一个 draft PR),而不是发明新模式。

---

## 6. 联系方式

- Issue 与讨论:GitHub
- 直接联系:**contact@z2net.com**

---

## 7. 快速参考 - 已接受的决策

| 主题        | 决策                                      |
| ----------- | ----------------------------------------- |
| 宏          | `#[bffi]`：shim（debug 直接 / release catch_unwind）+ bffi_meta_* 描述符 |
| `#[bffi]` 返回值 | 原始类型/bigint 经 out-param;`String`/`Vec<u8>`/`CopiedBuf`(及 `Option`)作为缓冲区句柄;组合类型(records/enums/`Vec<T>`/`Vec<Vec<u8>>` 及其 `Option`)走 wire 句柄;`Result<T, E: Into<BffiError>>` -> 转换后错误的状态(`status_u32()`) |
| 最低 Bun    | 1.4.2                                     |
| Rust/Cargo  | 1.98.0                                    |
| 句柄        | Generational Index + type-tag             |
| 错误格式    | `BffiError` = 代码 + 消息 + 来源 + rich 槽;领域错误通过 `From` 无损转换 |
| 边界字符串  | UTF-8 为规范编码(`bun:ffi cstring`)       |
| 表格并发    | 无锁;危险指针回收                          |
| UTF-8 校验  | SIMD(x86 SSSE3,aarch64 NEON);标量版作为参考 |
| 缓冲区      | 默认复制                                  |
| 零拷贝      | 仅通过 `bffi::unsafe_zero_copy`           |
| 事件循环    | `run()` 阻塞式排空;`pump()` 非阻塞排空;`marshal` - 错误线程路径(代码 12) |
| TS 类型 | IR（ModuleDef/FunctionDef/ClassDef）+ 确定性 render；export_name = bffi_ 前缀 |
| 组合类型(B1+B4) | Records/enums/`Vec<T>`(含 `Vec<Vec<u8>>`)支持 sync + async;带数据的 enum 变体走 kind 包络(`TAG_RECORD`:变体名 + 位置式 payload;TS `{ kind, ... }` 可辨识联合,判别字段 `kind`,tuple 字段 `_0`..),仅 unit 变体的 enum 保持 `TAG_STR` 字符串联合;`Option<Record>`/`Option<Vec<T>>` = 通过空缓冲区 0 句柄约定的 `\| null`;更深的嵌套会被拒绝 |
| 类型化错误(B3) | `#[derive(BffiError)]`:用户码 0x1000-0xFFFF 替换状态 13;variant = JS `e.name`,字段 = `e.payload`(TAG_RECORD);rich 访问器 best-effort;loader JSON 的 `errors` 表 |
| 流(B2)  | `#[bffi_stream]`:pull(`impl Iterator<Item = T> + Send`)或 push(`async fn(ctx: Ctx<T>, ...)`,bounded 256,背压)作为 JS `AsyncIterableIterator<T>`;`bffi_stream_next(handle, max)`(TAG_SEQ 缓冲,0 = 结束;14 = Pending 重试)+ `bffi_stream_drop` + `bffi_stream_set_wake`(经 event-loop 的唤醒 trampoline,best-effort);标签 0x0600;push 生产者交付 `Result` 项(`ctx.push(Ok/Err)`) |
| 类宏 | 基于 ObjectWrap（标签 0x0100-0x01FF）的 `#[bffi_class]`/`#[bffi_impl]`:pub 原始字段的 getter、`&self` 方法、自动生成 release;元数据拆分为 bffi_meta_<name> + bffi_meta_<name>_impl::CLASS;E005-E008 |
| 宏支持 | `bffi-macros::support`:proc-macro crate 的共享模型/映射/代码生成内部模块(无运行时代码、无 ABI) |
| Panic(生产) | 转换为 JS Error                           |
| Panic(开发) | 可以中止                                  |
| 兼容性      | 仅支持 Bun                                |
| 许可证      | MIT                                       |
| 门面         | `bffi`:扁平化再导出整个栈;`unsafe_zero_copy` 是唯一的零拷贝入口;宏展开默认引用 `::bffi::{core,types,dts,object,build,r#async}`(`crate = "<name>"` 重定向,`crate = "direct"` 选择 pre-merge 根) |
| 异步         | `#[bffi_async]`:spawn 包装函数(shim)返回任务句柄;N-worker 执行器;协作式取消 + 超时;经 event-loop 入队交付;tokio opt-in;标签 0x0500-0x05FF;组合类型返回走 wire 通道(`Promise<Record>` / `Promise<Vec<T>>` 经 `AsyncValue::Wire`,`Option` -> `Promise<... \| null>`);`E: Into<BffiError>` 契约 |
| 对象所有权 | 基于全局 `Registry` 的 `ObjectWrap<T>`(标签 0x0100-0x01FF);release 释放槽位 |
| 回调 | `register`/`revoke` + `bind_js_callback`;标签 0x0200-0x0201;错误线程 - 拒绝;`invoke_wait` 经 marshal 把回调投递到 JS 线程,可从任意原生线程调用,必带超时(`Timeout = 15`)- 两张表(原生闭包与 JS-bound 句柄) |
| 构建 ABI | 运行时导出（`bffi_error_*`、`bffi_buffer` 对、`bffi_types_free`）通过在用户 crate 中展开的 `bffi_runtime_abi!()` 生成；标签 0x0400-0x04FF；规范契约：bffi/CALLING-CONVENTION.md |
| 描述符 ABI | `FunctionDef`/`MethodDef` 携带 `AbiSig`（精确 C 宽度 + out 槽）；`FieldDef` 携带 getter 的 `export_name` + out；`ClassDef` 携带 `release_export` |
| Wire 编解码 | `bffi::types::wire`:统一的 `[tag][payload]` 表,服务异步负载与回调签名/参数/结果 |
| 回调 ABI | 通过 `bffi_callback_abi!()`（`bffi_callback_set_thread`/`_bind`/`_invoke`/`_revoke`）在用户 crate 生成的泛型导出；wire 编码；CALLING-CONVENTION.md §9 |
| Loader JSON | `bffi::build::loader_json`:由聚合的 `ModuleDef` 渲染的规范、确定性 JSON 模式 v1 |
| TS API 代码生成 | `bun bffi codegen <json> -o <ts>`:确定性渲染器;内嵌模式字面量;`ApiOf<>` 基于 `packages/bffi` 推导精确类型 |
| 平台分发 | napi-rs 风格的平台 npm 包(optionalDependencies 精确锁版本、`bffi pack`、resolvePlatformBinary) |
| 参考原生模块 | `crates/bffi-native` -> `@z2net/bffi-native` 平台包家族 |
