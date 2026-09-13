# AGENTS.md - Правила для AI-агентов и контрибьюторов

[English](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) | **[Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/AGENTS.md)** | [简体中文](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/AGENTS.md)

Этот файл определяет, как люди и AI-агенты должны работать над **bffi-rs**.

Репозиторий: https://github.com/z2net/bffi-rs
Контакт: contact@z2net.com

---

## 1. Назначение проекта

`bffi-rs` - фреймворк нативных привязок **только для Bun**, написанный на Rust.

Это аналог `napi-rs` для Bun, но:

- нацелен **только на Bun** (без совместимости с Node.js / Deno);
- **не** зависит от Node-API;
- использует `bun:ffi` и тонкий слой C ABI;
- строится **снизу вверх** из небольших крейтов.

Основные цели: безопасность на границе FFI, ясная модель владения, удобство разработки и долгосрочная сопровождаемость.

Читайте `DESIGN.md` перед внесением архитектурных изменений.

---

## 2. Жёсткие правила

1. **Только Bun**
   Не добавляйте слои совместимости с Node.js или Deno.

2. **Безопасность прежде всего**
   - Путь по умолчанию = копирование данных, никогда zero-copy.
   - Zero-copy разрешён только через `bffi::unsafe_zero_copy`.
   - Все функции `extern "C"` должны быть тонкими обёртками и работать под `catch_unwind`.

3. **Дескрипторы (handles)**
   Используйте индекс с поколением + тег типа (`u64`).
   Никогда не выставляйте наружу сырые ссылки Rust или сложные типы через C ABI.

4. **Паники**
   - Отладочные сборки могут прерывать процесс (так проще отлаживать).
   - Продакшн-сборки обязаны преобразовывать паники в JS `Error`.

5. **Минимальная версия Bun**
   `1.4.0`

6. **Версия Rust / Cargo**
   Проект закреплён на **Cargo / Rust 1.98.0**.
   Не поднимайте версию без явного решения и обновления CI.

7. **Никаких секретов в репозитории**
   Всё, что находится под `.grok`, `.claude`, `.codex`, `.opencode`, `.hermes`, `.mcp`, `.env`, ключи, токены и т.п., должно оставаться вне git (см. `.gitignore`).

8. **Лицензия**
   MIT. Сохраняйте SPDX-заголовки там, где это уместно.

---

## 3. Структура репозитория

```
bffi-rs/
├── AGENTS.md                      # этот файл
├── README.md
├── LICENSE
├── SECURITY.md
├── CONTACT.md
├── Cargo.toml                     # workspace
├── rust-toolchain.toml            # закреплён 1.98.0
├── package.json                   # Bun workspace / скрипты
├── tsconfig.json
├── .oxlintrc.json                 # конфигурация линтера
├── lefthook.yml                   # git-хуки (lint, fmt, commit-msg)
├── .gitignore
├── .github/
│   ├── ISSUE_TEMPLATE/
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── workflows/                 # CI (ci.yml) + релиз нативных пакетов (release-native.yml)
├── crates/
│   ├── bffi-core/                 # фундамент (дескрипторы, catch_unwind, ...)
│   ├── bffi-types/                # преобразование типов
│   ├── bffi-error/
│   ├── bffi-object/
│   ├── bffi-callback/
│   ├── bffi-class/
│   ├── bffi-dts/                  # генерация TypeScript .d.ts
│   ├── bffi-macros/
│   ├── bffi-macro-support/        # общие внутренности макросов (kinds, classify, codegen)
│   ├── bffi-event-loop/
│   ├── bffi-async/                # фьючерсы как JS Promises
│   ├── bffi-build/
│   ├── bffi-native/               # эталонная cdylib (runtime ABI; -> пакеты @z2net/bffi-native)
│   └── bffi/                      # публичный фасад
├── docs/
│   ├── DESIGN.md                  # архитектура и решения
│   ├── CONTRIBUTING.md
│   └── CODE_OF_CONDUCT.md
├── examples/                      # sqlite, async, event-loop, callbacks (каждый - набор e2e-тестов)
├── packages/                      # JS-сторона: bffi (@z2net/bffi), bffi-cli, native
└── scripts/
```

Новые крейты должны следовать схеме именования `bffi-*` и добавляться в workspace.

---

## 4. Процесс разработки

### Настройка

```bash
# Rust
rustup toolchain install 1.98.0
rustup default 1.98.0

# Bun
bun install
```

### Часто используемые команды

```bash
bun run lint          # oxlint
bun run typecheck     # tsc
bun run build         # собирает все четыре примера-крейта (release cdylib)
bun run test:e2e      # прогоняет примеры как наборы e2e-тестов (bun test examples)
bun run ci            # полный CI-паритет: lint, typecheck, fmt, clippy, тесты
cargo check
cargo test
cargo fmt
cargo clippy
```

### Стиль коммитов

Мы используем **Conventional Commits**:

```
feat: add generational handle table
fix: prevent panic across FFI boundary
docs: update DESIGN.md decisions
refactor(core): simplify catch_unwind helper
test: cover buffer copy path
chore: pin rust-toolchain to 1.98.0
```

Критические изменения (breaking changes) указываются через `BREAKING CHANGE:` в футере или `!` после типа.

### Ветвление и релизы

- `main` - продакшн-ветка; пул-реквесты в `main` создаёт только владелец проекта, из `dev/main`.
- `dev/main` - интеграционная ветка; вся работа над фичами попадает сюда через пул-реквесты.
- Фичи разрабатываются в ветках `dev/<feature>` (kebab-case), которые отпочковываются от `dev/main` и мержатся обратно в `dev/main`.
- Пул-реквест `dev/<feature>` → `dev/main` требует 1 одобрения и зелёного CI (`.github/workflows/ci.yml`; локально - `bun run ci`).
- Релизные теги `v<semver>` (аннотированные) ставятся только на `main` и только владельцем.

Полные правила: [docs/i18n/ru/CONTRIBUTING.md](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/CONTRIBUTING.md) → "Ветвление и релизы".

### Пул-реквесты

- Одно логическое изменение на пул-реквест.
- CI должен проходить.
- Обновляйте документацию при изменении поведения или публичного API.
- Ссылайтесь на связанные issues.

---

## 5. Правила для AI-агентов

Работая над этим репозиторием, агент **обязан**:

1. Прочитать `DESIGN.md` и этот файл перед крупными изменениями.
2. Делать небольшие, удобные для ревью диффы.
3. Никогда не коммитить секреты, личные AI-конфигурации или файлы `.env`.
4. Не вводить совместимость с Node/Deno.
5. Сохранять архитектуру снизу вверх (небольшие крейты → `bffi-rs`).
6. Сохранять модель безопасности (копирование по умолчанию, явный unsafe zero-copy, дескрипторы с поколениями).
7. Запускать `cargo fmt`, `cargo clippy` и тесты, когда это возможно.
8. Обновлять `DESIGN.md` или документацию, если меняется решение.

Если возникают сомнения насчёт архитектуры, лучше спросить (или открыть черновой PR), чем изобретать новый подход.

---

## 6. Контакт

- Issues и обсуждения: GitHub
- Прямой контакт: **contact@z2net.com**

---

## 7. Краткий справочник - принятые решения

| Тема          | Решение                                  |
| ------------- | ---------------------------------------- |
| Макрос        | `#[bffi]`: C-функция-обёртка (debug - без обёртки / release - под `catch_unwind`) + дескриптор `bffi_meta_*` |
| Возвраты `#[bffi]` | примитивы/bigint через выходной параметр; `String`/`Vec<u8>`/`CopiedBuf` (и `Option` от них) как дескрипторы буферов; композиты (records/enums/`Vec<T>`/`Vec<Vec<u8>>` и `Option` от них) как wire-дескрипторы; `Result<T, E: Into<BffiError>>` -> статус конвертированной ошибки (`status_u32()`) |
| Мин. Bun      | 1.4.0                                    |
| Rust/Cargo    | 1.98.0                                   |
| Дескрипторы   | Индекс с поколением + тег типа           |
| Формат ошибок | `BffiError` = код + сообщение + источник + rich-слот; доменные ошибки конвертируются без потерь через `From` |
| Строки на границе | Каноничный UTF-8 (`bun:ffi cstring`) |
| Таблицы       | Lock-free; освобождение слотов через hazard-указатели |
| Проверка UTF-8| SIMD (x86 SSSE3, aarch64 NEON) + скалярный эталон |
| Буферы        | Копирование по умолчанию                 |
| Zero-copy     | Только через `bffi::unsafe_zero_copy`    |
| Event loop    | `run()` опустошает очередь блокирующе; `pump()` - без блокировки; `marshal` - путь для вызова не из того потока (код 12) |
| TS-типы       | IR (ModuleDef/FunctionDef/ClassDef) + детерминированная генерация; export_name с префиксом `bffi_` |
| Композиты (B1+B4) | Records/enums/`Vec<T>` (включая `Vec<Vec<u8>>`) sync + async; `Option<Record>`/`Option<Vec<T>>` = `\| null` через конвенцию 0-хендла пустого буфера; более глубокая вложенность отклоняется |
| Typed errors (B3) | `#[derive(BffiError)]`: юзер-коды 0x1000-0xFFFF заменяют статус 13; вариант = JS `e.name`, поля = `e.payload` (TAG_RECORD); rich-аксессоры best-effort; таблица `errors` в loader JSON |
| Streams (B2)  | `#[bffi_stream]`: pull (`impl Iterator<Item = T> + Send`) или push (`async fn(ctx: Ctx<T>, ...)`, bounded 256, backpressure) как JS `AsyncIterableIterator<T>`; `bffi_stream_next(handle, max)` (TAG_SEQ буфер, 0 = конец; 14 = Pending) + `bffi_stream_drop` + `bffi_stream_set_wake` (wake-трамплин через event loop, best-effort); тег 0x0600; push-продюсеры доставляют `Result`-элементы (`ctx.push(Ok/Err)`) |
| Макросы классов | `#[bffi_class]`/`#[bffi_impl]` поверх ObjectWrap (теги 0x0100-0x01FF): геттеры полей, методы `&self`, автоматический release; метаданные разделены: bffi_meta_<name> + bffi_meta_<name>_impl::CLASS; диагностика E005-E008 |
| Macro support | `bffi-macro-support`: общие модель/маппинг/кодогенерация для крейтов-проц-макросов (bffi-macros, bffi-class); служебный крейт - без кода времени выполнения и ABI |
| Паника (prod) | Преобразуется в JS Error                 |
| Паника (dev)  | Может прерывать процесс (abort)          |
| Совместимость | Только Bun                               |
| Лицензия      | MIT                                      |
| Фасад         | `bffi`: плоские реэкспорты стека; `unsafe_zero_copy` - единственная точка zero-copy; раскрытия макросов по умолчанию ссылаются на `::bffi::{core,types,dts,object,build,r#async}` (`crate = "<name>"` перенаправляет, `crate = "direct"` - pre-merge корни) |
| Async         | `#[bffi_async]`: функция запуска возвращает дескриптор задачи; пул потоков-исполнителей; кооперативная отмена + таймауты; разрешение промиса доставляется через event-loop enqueue; tokio подключается опционально; теги 0x0500-0x05FF; композитные возвраты едут через wire-канал (`Promise<Record>` / `Promise<Vec<T>>` через `AsyncValue::Wire`, `Option` -> `Promise<... \| null>`); контракт `E: Into<BffiError>` |
| Владение объектами | `ObjectWrap<T>` поверх глобального `Registry` (тег 0x0100-0x01FF); release освобождает слот |
| Колбэки | `register`/`revoke` + `bind_js_callback`; теги 0x0200-0x0201; вызов не из того потока - отказ; `invoke_wait` маршалит колбэк на JS-поток с ЛЮБОГО нативного потока с обязательным таймаутом (`Timeout = 15`) - обе таблицы (нативные замыкания и JS-bound хендлы) |
| Runtime ABI | Экспорты времени выполнения (`bffi_error_*`, пара `bffi_buffer`, `bffi_types_free`) через `bffi_runtime_abi!()` в крейте пользователя; теги 0x0400-0x04FF; канонический контракт: bffi/CALLING-CONVENTION.md |
| ABI дескрипторов | `AbiSig` (точные C-ширины + выходной слот) в `FunctionDef`/`MethodDef`; `export_name` геттера + выходной слот в `FieldDef`; `release_export` в `ClassDef` |
| Формат обмена (wire) | `bffi_types::wire`: одна таблица `[tag][payload]` для async-результатов и сигнатур/аргументов/результатов колбэков |
| Callback ABI | Универсальные экспорты через `bffi_callback_abi!()` (`bffi_callback_set_thread`/`_bind`/`_invoke`/`_revoke`) в крейте пользователя; wire-кодирование; CALLING-CONVENTION.md §9 |
| Loader JSON | `bffi_build::loader_json`: канонический детерминированный JSON схемы v1 из агрегированного `ModuleDef` |
| Генерация TS API | `bun bffi codegen <json> -o <ts>`: детерминированная генерация; встраивает литерал схемы; `ApiOf<>` выводит точные типы поверх `packages/bffi` |
| Платформенная дистрибуция | Платформенные npm-пакеты в стиле napi-rs (точные пины в optionalDependencies, `bffi pack`, resolvePlatformBinary) |
| Эталонный нативный модуль | `crates/bffi-native` -> семейство платформенных пакетов `@z2net/bffi-native` |
