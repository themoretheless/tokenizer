# План оптимизации tokenizer

## Контекст
- **Проект:** themoretheless/tokenizer — lossless multi-language syntax engines
- **Дата:** 2026-10-02

## Текущее состояние
### Тулчейн
- Rust nightly (2026-09-27) - rustc 1.101.0-nightly
- Самостоятельно обновлённый Cargo.lock

### Benchmarks
- Кастомный JSON benchmark (`benches/json_bench.rs`)
- Замеряет: lex, parse, tokenize vs serde_json
- Дatasets: small, unicode, large, deep, malformed
- Airbug-bench **не интегрирован** (только внешний skill)

## Точки оптимизации

### 1. Compiler flags (.cargo/config.toml)
```toml
[profile.bench]
overflow-checks = false
lto = "fat"
codegen-units = 1
opt-level = 3
```

**Ожидаемый выигрыш:** 5-15%

### 2. SIMD оптимизации парсера
- Парсинг 16 байт за раз с `std::simd`
- Блочная обработка идентификаторов/строк
- `#![feature(portable_simd)]`

**Ожидаемый выигрыш:** 2-4x для large datasets

### 3. Zero-copy parsing
- Избежать `to_owned()` где можно
- String interning для common tokens (`ark`, `dashmap`)
- Использовать `&str` вместо `String` где возможно

**Ожидаемый выигрыш:** 10-30% меньше аллокаций

### 4. Parallel parsing
- Rayon для параллельного парсинга JSON arrays
- Параллельная обработка элементов массива

```rust
use rayon::prelude::*;
array.par_iter().map(|item| parse_item(item)).collect()
```

**Ожидаемый выигрыш:** 1.8-3x на многоядерных системах

### 5. Cache-aware data structures
- Arena allocator (`bumpalo`, `typed-arena`)
- Array-based trees для nested structures
- Pool allocations вместо fragment

**Ожидаемый выигрыш:** 5-15% за счёт cache locality

### 6. Algorithmic improvements
- Lazy evaluation для редко используемых полей
- Stream parser вместо pull parser
- Precomputed transition tables для FSM
- Branch prediction friendly кодирование состояний

## Приоритеты

### Высокий приоритет (Quick wins):
1. Compiler flags (.cargo/config.toml)
2. Zero-copy improvements в критических путях

### Средний приоритет:
3. SIMD оптимизации для JSON parser
4. String interning для common tokens

### Долгосрочные:
5. Parallel parsing infrastructure
6. Полный memory profiling + рефакторинг

## Метрики успеха
- Уменьшение ns/op на JSON bench
- Снижение allocation rate
- Улучшение MiB/s throughput

## Следующие шаги
1. Создать `.cargo/config.toml`
2. Прогнать baseline bench
3. Apply SIMD changes
4. Добавить string interning test
5. Измерить результаты
