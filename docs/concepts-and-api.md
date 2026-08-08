# Что такое лексер, парсер, токенизация, подсветка и ошибки

Короткий глоссарий и как это ложится на `themoretheless-tokenizer`.

## Слои (снизу вверх)

```text
текст (UTF-8)
   │
   ▼
┌──────────────────┐
│  LEXER (лексер)  │  режет на токены: { " name " : 1 }
│  syntax tokens   │  lossless: concat(tokens) == source
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  PARSER (парсер) │  собирает дерево: object → member → number
│  AST / CST       │  recovery: битый ввод всё равно даёт куски дерева
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  HIGHLIGHT       │  токены для цвета: property vs string, key vs value
│  semantic tokens │  учитывает структуру (не только «что за символ»)
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  DIAGNOSTICS     │  ошибки/предупреждения со span, без «молчаливого truncate»
│  поиск ошибок    │  подсветка при этом продолжается
└──────────────────┘
```

| Термин | По-русски | Что делает | У тебя в коде |
|--------|-----------|------------|----------------|
| **Lexer** | Лексер | Делит текст на токены | `json::lex`, API `Source::lex` / `syntax` |
| **Tokenizer** | Токенизатор | В продукте = **highlight** (не лексер) | `Source::tokenize` / `highlight` |
| **Parser** | Парсер | Строит структуру (AST) | `json::parse` (typed) |
| **AST** | Дерево значений | Object/Array/… | `json::Value` |
| **CST** | Concrete syntax tree | Дерево + все токены | `json::syntax_tree` |
| **Semantic tokens** | Подсветка | Kind с учётом контекста | `json::tokenize`, `Source::highlight` |
| **Diagnostics** | Поиск ошибок | code + message + span | `Source::errors`, lex/parse diagnostics |
| **Span** | Диапазон | UTF-8 bytes `[start, end)` | `Span` |

### Важно

1. **Токены ≠ только подсветка.** Лексер даёт exact syntax kinds; highlight может переименовать/уточнить их.
2. **Ошибки soft.** Невалидный JSON всё равно токенизируется; diagnostics объясняют, что не так.
3. **Парсер не обязателен для подсветки URL** сегодня (capabilities: LEX|VALIDATE). JSON — полный stack.
4. **Typed JSON AST** не прячется в dyn API: для структуры используй `json::parse`, для редактора — `Source`.

## Удобный API (facade)

```rust
use themoretheless_tokenizer::api::Source;

// Подсветка JSON
let hi = Source::new("json", r#"{"a":1}"#)
    .dialect("strict")
    .highlight()?;

// Только лексер (syntax tokens)
let lex = Source::new("json", source).dialect("jsonc").syntax()?;

// Только ошибки
let errs = Source::new("json", source).errors()?;

// URL
let url = Source::new("url", "https://example.com").highlight()?;
```

Однострочники:

```rust
use themoretheless_tokenizer::api::quick;

let a = quick::highlight_json(r#"[1,2,3]"#)?;
let b = quick::highlight_url("https://x.test/y")?;
let c = quick::errors("json", "{,}")?;
```

Typed JSON (когда нужно дерево, не только цвет):

```rust
use themoretheless_tokenizer::json::{parse, ParseOptions};

let tree = parse(r#"{"n":1}"#);
let value = tree.value();
```

## Prelude

```rust
use themoretheless_tokenizer::api::prelude::*;
```

## Что не смешивать

| Не делай | Почему |
|----------|--------|
| Ждать AST через `Source` | Dyn facade отдаёт string kinds, не `json::Value` |
| Путать `syntax` и `highlight` | Property vs string видно только на semantic/highlight |
| Считать `valid == false` «нет токенов» | Токены есть; valid = нет error diagnostics |
| Искать «полный Python» в registry | Пока плагины: json, url |

## Дальше

- Контракт плагинов: `docs/plugin-api-design.md`
- Новый язык = новый crate + `HostLanguage` + feature на facade
