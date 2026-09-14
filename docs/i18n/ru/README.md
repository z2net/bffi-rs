# bffi-rs

<div align="center">

[![Bun](https://img.shields.io/badge/Bun-%3E%3D1.4.0-F472B6?logo=bun&logoColor=white)](https://bun.sh)
[![Rust](https://img.shields.io/badge/Rust-1.98.0-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-3DA639?logo=opensourceinitiative&logoColor=white)](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
[![GitHub Issues](https://img.shields.io/github/issues/z2net/bffi-rs)](https://github.com/z2net/bffi-rs/issues)
[![GitHub Pull Requests](https://img.shields.io/github/issues-pr/z2net/bffi-rs)](https://github.com/z2net/bffi-rs/pulls)

[English](https://github.com/z2net/bffi-rs/blob/main/README.md) | **Русский** | [简体中文](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/README.md)

</div>

Фреймворк привязок для Bun - аналог `napi-rs` для [Bun](https://bun.sh), построенный на `bun:ffi` и тонком C ABI. Написан на Rust, снизу вверх, из небольших специализированных крейтов.

Архитектура описана в [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md), инженерные правила проекта - в [AGENTS.md](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md).

## Документация

- [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md) - архитектура и принятые решения
- [crates/bffi/CALLING-CONVENTION.md](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/CALLING-CONVENTION.md) - контракт C ABI (все пересечения границы, включая callback-экспорты)
- [docs/CONTRIBUTING.md](https://github.com/z2net/bffi-rs/blob/main/docs/CONTRIBUTING.md) - как вносить изменения (ветки, коммиты, PR)
- [AGENTS.md](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) - правила для людей и ИИ-агентов
- [packages/bffi](https://github.com/z2net/bffi-rs/blob/main/packages/bffi) - `@z2net/bffi`: типизированный лоадер + пайплайн сборки (подробности в его README)
- [packages/bffi-cli](https://github.com/z2net/bffi-rs/blob/main/packages/bffi-cli) - `@z2net/bffi-cli`: CLI `bffi` (init, build, check, doctor, codegen, pack, fetch)
- [packages/native](https://github.com/z2net/bffi-rs/blob/main/packages/native) - `@z2net/bffi-native`: эталонный нативный модуль (семейство платформенных npm-пакетов)
- [bffi-examples](https://github.com/z2net/bffi-examples) - примеры-модули, каждый из них ещё и набор e2e-тестов (sqlite, records, streams, errors, async, event-loop, callbacks, workers, wry)
- [SECURITY.md](https://github.com/z2net/bffi-rs/blob/main/SECURITY.md) - политика безопасности
- [CONTACT.md](https://github.com/z2net/bffi-rs/blob/main/CONTACT.md) - контакты

## Требования

- [Bun](https://bun.sh) >= 1.4.0 (проверяется в рантайме `@z2net/bffi` и CLI `bffi`)
- Rust 1.98.0 (закреплён через `rust-toolchain.toml`; rustup установит его сам)
- bash (для хука commit-msg; предустановлен на macOS/Linux, на Windows - Git Bash)

## Состав

| Часть | Назначение |
| ---- | ------- |
| `crates/bffi` | **Публикуемый крейт** ([crates.io/crates/bffi](https://crates.io/crates/bffi)): весь стек как feature-gated модули (core, types, error, object, dts, build, callback, event-loop, async) + фасад + реэкспорт макросов |
| `crates/bffi-macros` | Proc-macro крейт ([crates.io/crates/bffi-macros](https://crates.io/crates/bffi-macros)): `#[bffi]`, `#[bffi_async]`, `#[bffi_class]`, `#[bffi_impl]`, `#[bffi_constructor]` |
| `crates/bffi-native` | Эталонная cdylib (`add`/`shout`/`version` + рантайм ABI); источник семейства платформенных пакетов `@z2net/bffi-native` |
| `packages/bffi` | Bun-only JS-пакет интеграции: конфиг, полный пайплайн (build → json → api.gen), типизированный лоадер (npm: `@z2net/bffi`) |
| `packages/bffi-cli` | CLI `bffi`: init, build, codegen, pack, fetch, check, doctor (npm: `@z2net/bffi-cli`) |

## Быстрый старт

```sh
bun install          # зависимости + git-хуки (lefthook)
bun run build        # собирает эталонную cdylib (release)
bun run test:js      # прогоняет юнит-тесты пакетов (bun test packages)
bun run check        # oxlint + tsc + cargo check
bun run ci           # полный CI-паритет: lint, typecheck, fmt, clippy, тесты, JS-тесты
```

Для своего нативного модуля зависите на [`bffi`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi) (фасад: одна зависимость на весь стек) и - при использовании атрибутных макросов - на отдельных крейтах `bffi-core`/`bffi-types`/`bffi-dts`, которые упоминают их раскрытия.

## Генерируемый типизированный API

Дескрипторы `#[bffi]` - единственный источник истины: бинарник `emit-json` внутри крейта пишет `.bffi/bffi.api.json` (схема v1) из агрегированного `ModuleDef`, а пайплайн `@z2net/bffi` делает остальное - валидирует, генерирует `.bffi/api.gen.ts`, находит и `dlopen`-ит библиотеку. Байты детерминированы: файл можно коммитить и спокойно смотреть в diff.

```ts
import { bffi } from "@z2net/bffi";
import type { Api } from "./.bffi/api.gen.ts";

const api: Api = await bffi();       // один вызов: build -> json -> gen -> dlopen
api.add(1, 2);                       // number, типизировано; ошибки бросают JS Error
const counter = new api.counter(10); // классы: FinalizationRegistry + release()
await api.compute(21);               // `#[bffi_async]` -> Promise
```

Пайплайн, его конфигурация (`.bffi/bffi.json`) и все тонкости описаны в [`packages/bffi`](https://github.com/z2net/bffi-rs/blob/main/packages/bffi); разобранные примеры живут в отдельном репозитории [bffi-examples](https://github.com/z2net/bffi-examples) - каждый пример это самостоятельный крейт и набор e2e-тестов против опубликованных пакетов (входной пример - [sqlite](https://github.com/z2net/bffi-examples/tree/main/sqlite), полный пайплайн поверх rusqlite).

## Конвенции

- Conventional Commits проверяются хуком `commit-msg` (`scripts/commit-msg.sh`).
- Pre-commit запускает oxlint, `tsc --noEmit`, `cargo fmt --check` и clippy.
- Pre-push запускает тесты workspace.
- GitHub Actions CI (`.github/workflows/ci.yml`) запускается на каждом pull request и на пушах в `main` / `dev/main` (Rust-матрица: ubuntu / windows / macos, плюс JS-джоба); `bun run ci` остаётся командой локального паритета.

## Лицензия

[MIT](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
