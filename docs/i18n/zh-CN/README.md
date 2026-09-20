# bffi-rs

<div align="center">

[![Bun](https://img.shields.io/badge/Bun-%3E%3D1.4.2-F472B6?logo=bun&logoColor=white)](https://bun.sh)
[![Rust](https://img.shields.io/badge/Rust-1.98.0-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-3DA639?logo=opensourceinitiative&logoColor=white)](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
[![GitHub Issues](https://img.shields.io/github/issues/z2net/bffi-rs)](https://github.com/z2net/bffi-rs/issues)
[![GitHub Pull Requests](https://img.shields.io/github/issues-pr/z2net/bffi-rs)](https://github.com/z2net/bffi-rs/pulls)

[English](https://github.com/z2net/bffi-rs/blob/main/README.md) | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/README.md) | **简体中文**

</div>

Bun 的绑定框架 - [Bun](https://bun.sh) 的 napi-rs 等价物,构建于 `bun:ffi` 与一层薄 C ABI 之上。用 Rust 编写,自底向上,由多个小而专一的 crate 组成。

架构见 [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md),项目工程规则见 [AGENTS.md](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md)。

## 文档

- [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md) - 架构与决策
- [crates/bffi/CALLING-CONVENTION.md](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/CALLING-CONVENTION.md) - C ABI 契约(每一次跨界,含回调导出)
- [docs/CONTRIBUTING.md](https://github.com/z2net/bffi-rs/blob/main/docs/CONTRIBUTING.md) - 如何贡献(分支、提交、PR)
- [AGENTS.md](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) - 面向人类与 AI 代理的工程规则
- [packages/bffi](https://github.com/z2net/bffi-rs/blob/main/packages/bffi) - `@z2net/bffi`:类型化加载器 + 构建流水线(见其 README)
- [packages/bffi-cli](https://github.com/z2net/bffi-rs/blob/main/packages/bffi-cli) - `@z2net/bffi-cli`:`bffi` CLI(init、build、check、doctor、codegen、pack、fetch)
- [packages/native](https://github.com/z2net/bffi-rs/blob/main/packages/native) - `@z2net/bffi-native`:参考原生模块(平台 npm 包家族)
- [bffi-examples](https://github.com/z2net/bffi-examples) - 示例模块,每一个同时也是一个 e2e 测试套件(sqlite、records、streams、errors、async、event-loop、callbacks、workers、wry)
- [SECURITY.md](https://github.com/z2net/bffi-rs/blob/main/SECURITY.md) - 安全策略
- [CONTACT.md](https://github.com/z2net/bffi-rs/blob/main/CONTACT.md) - 联系方式

## 环境要求

- [Bun](https://bun.sh) >= 1.4.2(运行时由 `@z2net/bffi` 与 `bffi` CLI 强制检查)
- Rust 1.98.0(经 `rust-toolchain.toml` 锁定;rustup 会自动安装)
- bash(commit-msg 钩子需要;macOS/Linux 预装,Windows 上为 Git Bash)

## 组件

| 部分 | 用途 |
| ---- | ------- |
| `crates/bffi` | **已发布的 crate**([crates.io/crates/bffi](https://crates.io/crates/bffi)):整个技术栈作为 feature 门控模块(core、types、error、object、dts、build、callback、event-loop、async)+ 门面 + 宏再导出 |
| `crates/bffi-macros` | proc-macro crate([crates.io/crates/bffi-macros](https://crates.io/crates/bffi-macros)):`#[bffi]`、`#[bffi_async]`、`#[bffi_class]`、`#[bffi_impl]`、`#[bffi_constructor]` |
| `crates/bffi-native` | 参考 cdylib(`add`/`shout`/`version` + 运行时 ABI);`@z2net/bffi-native` 平台包家族的源头 |
| `packages/bffi` | 仅限 Bun 的 JS 集成包:配置、完整流水线(build → json → api.gen)、类型化加载器(npm: `@z2net/bffi`) |
| `packages/bffi-cli` | `bffi` CLI:init、build、codegen、pack、fetch、check、doctor(npm: `@z2net/bffi-cli`) |

## 快速开始

```sh
bun install          # 安装依赖 + git 钩子(lefthook)
bun run build        # 构建参考 cdylib(release)
bun run test:js      # 运行包的单元测试(bun test packages)
bun run check        # oxlint + tsc + cargo check
bun run ci           # 完整 CI 对齐:lint、typecheck、fmt、clippy、测试、JS 测试
```

编写原生模块时,依赖 [`bffi`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi)
(门面:一个依赖覆盖整个栈);使用属性宏时,还需依赖其展开所引用的
`bffi-core`/`bffi-types`/`bffi-dts` 等 crate。

## 生成的 TypeScript API

`#[bffi]` 描述符是唯一事实来源:crate 的 `emit-json` 二进制从聚合的
`ModuleDef` 写出 `.bffi/bffi.api.json`(schema v1),其余步骤由
`@z2net/bffi` 流水线完成 - 校验、生成 `.bffi/api.gen.ts`、解析并
`dlopen` 动态库。逐字节确定,可放心提交与 diff。

```ts
import { bffi } from "@z2net/bffi";
import type { Api } from "./.bffi/api.gen.ts";

const api: Api = await bffi();       // one call: build -> json -> gen -> dlopen
api.add(1, 2);                       // number, typed; errors throw JS Errors
const counter = new api.counter(10); // classes: FinalizationRegistry + release()
await api.compute(21);               // `#[bffi_async]` -> Promise
```

流水线、它的配置(`.bffi/bffi.json`)以及每一处细节都记录在
[`packages/bffi`](https://github.com/z2net/bffi-rs/blob/main/packages/bffi);
完整的实战示例位于独立仓库
[bffi-examples](https://github.com/z2net/bffi-examples) - 每个示例
在那里都是独立的 crate,同时也是一个针对已发布包的 e2e 测试套件
(入门示例是 [sqlite](https://github.com/z2net/bffi-examples/tree/main/sqlite),
在 rusqlite 之上跑通完整流水线)。

## 约定

- Conventional Commits 由 `commit-msg` 钩子(`scripts/commit-msg.sh`)强制执行。
- pre-commit 运行 oxlint、`tsc --noEmit`、`cargo fmt --check` 和 clippy。
- pre-push 运行 workspace 测试。
- GitHub Actions CI(`.github/workflows/ci.yml`)在每个 pull request 以及向 `main` / `dev/main` 的推送上运行(Rust 矩阵:ubuntu / windows / macos,外加一个 JS 作业);`bun run ci` 仍是本地的对齐命令。

## 许可证

[MIT](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
