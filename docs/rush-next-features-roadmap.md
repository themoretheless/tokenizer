# Дорожная карта развития языка Rush: итоги и следующие шаги

Этот документ фиксирует уже внедрённые языковые заимствования (Verse, WGSL, Gleam/Elixir, Cue/KCL) и детальный план следующего пакета возможностей для `crates/tokenizer-rush`.

---

## 1. Выполненные этапы (в `main`)

### Этап 1 — Фишки из Verse ([PR #55](https://github.com/themoretheless/tokenizer/pull/55), коммит `e678f55`)
1. **Транзакционный блок отказа `try { ... }` с `O(dirty)` undo-логом:**
   - Ленивый write-barrier журнал отката грязных ячеек `mut`, созданных внутри блока корутин и загруженных модулей при срабатывании `?` на `None()` / `Err(e)` или возврате `None` / `Err` из блока.
   - Транзакционный хост-вызов `ScriptInstance::call_transactional` (`OwnedScriptInstance::call_transactional` / `call_values_transactional`) и явный контракт чистоты `HostRegistration::with_transactional(true)` (диагностика `non-transactional-host`).
2. **Опровержимое связывание `let <pattern> = expr else { ... }` и `if let`:**
   - Статический контроль расходимости ветки `else` (`let-else-must-diverge`: `return`, `break`, `continue`, `?`), изоляция частичных связываний составных паттернов.
   - `if let` (включая цепочки `else if let`) как оператор и как выражение в AST-рантайме, корутинах и Bytecode VM.
3. **Структурная конкурентность `await task_all([...])` и `await task_race([...])`:**
   - Кооперативные комбинаторы в `CoroutineScheduler` (`WakeRequest::All` и `WakeRequest::Race`) с автоматической каскадной отменой проигравших или соседних задач при ошибке и детекцией циклов ожидания.

### Этап 2 — Фишки из WGSL, Gleam/Elixir и Cue/KCL ([PR #57](https://github.com/themoretheless/tokenizer/pull/57), коммит `ac7835c`)
1. **WGSL-style Vector Swizzles (чтение и запись `xyzw` / `rgba`):**
   - Чтение масок длины `1..=4` на `vec2`, `vec3`, `vec4` в пространствах `xyzw` и `rgba` без их смешивания (`v.zyx`, `c.bgr`, `v.xxxx`).
   - Прямая и составная запись (`v.xz = vec2(9, 8)`, `c.rgb *= 0.5`, вложенные пути `p.pos.zx = ...`), статический и runtime-запрет дублирующихся каналов при записи (`swizzle-assignment`), откат внутри `try { ... }` и полный паритет AST / Bytecode VM ([tests/swizzles.rs](../crates/tokenizer-rush/tests/swizzles.rs)).
2. **Функциональное обновление `{ ..base, field: val }` и плейсхолдер `_` в пайплайнах:**
   - Немутирующее слияние записей и номинальных структур слева направо (`{ ..p, x: 10 }`, `{ ..r1, ..r2, c: 3 }`, `Point({ ..p, x: 10 })`) с сохранением номинального типа, приведением типов полей, проверкой `where`-ограничений, поддержкой `rename_project_symbol` и батчингом `Opcode::RecordUpdate` в Bytecode VM.
   - Подстановка входного значения пайплайна в явные слоты `_` (`x | clamp(5, 0, _)`, `v | dot(_, _)`) с однократным вычислением входа и выводом типов параметров ([tests/spread_and_pipe.rs](../crates/tokenizer-rush/tests/spread_and_pipe.rs)).
3. **Контекстные `where`-ограничения на `param` и полях `struct`:**
   - Контекстное ключевое слово `where <bool-expr>` (не резервирующее имя `where` для пайплайнов и идентификаторов).
   - Проверка на `param` при старте скрипта, переопределении хост-входов в `Program::instantiate`, в корутинах, Bytecode VM и атомарно в `ModelInstance::set_parameter` до пересчёта зависимых узлов.
   - Межполевые инварианты и вызовы функций модуля в полях `struct`, проверяемые при конструировании, spread-обновлении и мутации полей с атомарным откатом слота при нарушении ([tests/where_constraints.rs](../crates/tokenizer-rush/tests/where_constraints.rs)).

---

## 2. Предложения на следующий этап (Топ-6)

### 1. Field punning `{ x, y }` + паттерны деструктуризации в `for <pattern> in ...` *(Rust / Gleam)*
- **Проблема:** в [`StmtKind::For`](../crates/tokenizer-rush/src/ast.rs) сейчас допускается только одиночное имя `binding: Name<'s>`, хотя в `let`, `let ... else` и параметрах функций уже работают произвольные неопровержимые шаблоны. А в литералах записей, конструкторах `struct`, `{ ..base }` и record-шаблонах требуется дублировать `Point({ x: x, y: y })` и `let { x: x, y: y } = p`.
- **Решение:**
  ```rush
  let p = Point({ x, y })
  let next = { ..p, x }
  for (idx, { x, y }) in indexed_points {
      total += x + y
  }
  ```
- **Интеграция с редактором:** в [`rename_project_symbol`](../crates/tokenizer-rush/src/project.rs) переименование поля структуры на сокращённой записи `{ x }` автоматически раскрывает её в `{ new_field: x }`, а переименование локальной переменной — в `{ x: new_var }`.

### 2. Блочные ветви `{ ... }` и ИЛИ-паттерны `A | B` в `match` + блочные лямбды *(Rust / Roc)*
- **Проблема:** в [`MatchArm`](../crates/tokenizer-rush/src/ast.rs) и [`ExprKind::Lambda`](../crates/tokenizer-rush/src/ast.rs) после `=>` сейчас допускается только одиночное выражение, а одинаковые ветки `match` нельзя объединить через `|`.
- **Решение:**
  ```rush
  let speed = match state {
      State.Idle() | State.Paused() => 0.0,
      State.Moving(vel) => {
          let mag = clamp(length(vel), 0.0, max_speed)
          mag * boost
      },
  }
  ```
- **Детали:** проверка совпадения набора и типов связанных переменных во всех альтернативах `P1 | P2`, учёт ИЛИ-паттернов в проверке исчерпанности (`exhaustiveness`) и поддержка в AST, корутинах и Bytecode VM.

### 3. Сериализация и hot-reload богатого состояния в [`StateValue`](../crates/tokenizer-rush/src/runtime/instance.rs) *(Erlang / Godot)*
- **Проблема:** `export_state` / `import_state` сейчас поддерживают только примитивы, списки и сырые `Record`, отклоняя `vec2/3/4`, `quat`, `mat4`, кортежи, `Option`/`Result` и пользовательские `struct` / `enum` (см. раздел «Границы» в [rush-language-spec.md](rush-language-spec.md)).
- **Решение:** расширить `StateValue` вариантами для математических типов, кортежей, `Option`/`Result` и номинальных `Struct` / `Enum`. При `import_state` в обновлённый скрипт атомарно валидировать типы полей, варианты и `where`-контракты структур (с откатом при несовместимом снапшоте).

### 4. Значения по умолчанию у полей `struct` и параметров `fn` *(Kotlin / KCL / Cue)*
- **Проблема:** конструкторы структур и функции требуют указывать все поля и аргументы явно даже для опциональных настроек.
- **Решение:**
  ```rush
  struct Material {
      roughness: f64 = 0.5 where roughness >= 0.0 && roughness <= 1.0,
      metallic: f64 = 0.0 where metallic >= 0.0 && metallic <= 1.0,
      tint: vec3 = vec3(1, 1, 1),
  }
  let glass = Material({ roughness: 0.05 })
  fn extrude(profile: polygon, height: f64 = 10.0) -> mesh { ... }
  ```
- **Синергия с `StateValue`:** при `import_state` новые поля структуры, появившиеся при hot-reload скрипта, автоматически заполняются своими значениями по умолчанию и проходят проверку `where`.

### 5. Короткозамкнутый оператор `??` (unwrap-or) для `Option[T]` *(Swift / Kotlin)*
- **Проблема:** для получения значения из `Option[T]` с запасным значением по умолчанию сейчас нужен `match` или `if let`.
- **Решение:**
  ```rush
  let speed = override_speed ?? default_speed
  let target = primary ?? secondary ?? compute_fallback()
  ```
- Ленивое вычисление правой части только при `None()`, статическое сужение `Option[T]` + `T` -> `T` (и `Option[T]` + `Option[T]` -> `Option[T]`).

### 6. Размерные числовые литералы `90deg`, `1.5rad`, `250ms`, `2.5s` *(F# / WGSL / OpenSCAD)*
- **Решение:** лексическая/парсерная поддержка типизированных суффиксов без пробела:
  - `90deg` и `0.5rad` конструируют `Value::Angle` (`ValueType::Angle`) напрямую без вызовов `degrees(90)` / `radians(0.5)`.
  - `250ms` и `1.5s` переводятся в секунды (`f64`) для таймеров корутин (`await Wait.After(250ms)`).

---

## 3. Рекомендуемый следующий пакет (Топ-3)

1. **Field punning `{ x, y }` + паттерны в `for <pattern> in ...`** (с умным раскрытием при `rename_project_symbol`).
2. **Блочные ветви `{ ... }` и ИЛИ-паттерны `A | B` в `match` + блочные лямбды + оператор `??` для `Option[T]`**.
3. **Дефолты полей `struct` и параметров `fn` + полная поддержка `struct`/`enum`/`vecN`/`quat`/`mat4`/`Option`/`Result`/`tuple` в `StateValue` (`export_state` / `import_state`)** с проверкой `where`-инвариантов и автозаполнением дефолтных полей при миграции состояния.
