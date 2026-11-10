# Язык Forma

Один и тот же `.ui` / `.forma` разбирается компилятором Studio и генератором Rust-форм.
Studio вычисляет выражения по данным предпросмотра. Сгенерированная форма
подписывается на `Property<T>` и вычисляет их в Rust при изменении модели.
Интерактивный векторный предпросмотр записывает изменения привязанных полей
в своё состояние; пользовательская Rust ViewModel запускается кнопкой приложения.

## Контракт компонента и объявления

```text
use './theme.forma';

enum Tone { Accent, Danger }

struct Item {
    id: Int;
    #[range(0, 100000)]
    price: Number;
    label: String;
    badge: String?;
}

component Notice : Button {
    required prop title: String;
    prop tone: Tone = Tone.Accent;
    prop item: Item?;
    state expanded: Bool = false;
    let headerText = '${props.title} (${state.expanded ? "открыто" : "скрыто"})';
    tokens spacing { md: 12px; }
    i18n ru { empty: 'Нет данных'; }
    style card { padding: tokens.spacing.md; }
    shortcut 'Ctrl+Enter' -> events.open(props.title);

    event open(title: String);

    text: headerText;
    Row {
        apply card;
        slot leading(tone: props.tone) {
            Icon { width: 16; height: 16; }
        }
    }
}
```

`required prop` требует значение у экземпляра или в наследнике. Резервное
значение движка не удовлетворяет обязательному параметру. Обычный `prop`
получает значение после `=`; необязательный `T?` без значения получает `null`.
Старые объявления вида `text: '';` сохраняются. Строгий публичный контракт
задаётся через `prop`, локальное реактивное состояние — через `state`,
вычисляемые именованные выражения — через `let`.

Типы: `String`, `Bool` (`Boolean`), `Number` (`Float`), `Int`, `Color`,
`Length`, `Duration`, `Angle`, `Asset`, коллекции `List` / `List<T>`, словари
`Map` / `Map<V>` / `Record<String, V>`, `Object`, `Any`, объединения `A | B`,
объявленные `enum` / `struct` и необязательные варианты `T?`.
Числа должны быть конечными. `Int` проверяется как целое число, `Color` —
как hex-цвет (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`), `Length` — неотрицательное
число или единица `px`, `%`, `rem`, `em`, `vw`, `vh`, `vmin`, `vmax`, `ch`, `fr`,
`Duration` — значение в `ms` или `s`, `Angle` — число или `deg`, `rad`, `turn`.
`Asset` обозначает строку пути; проверка конкретного ресурса происходит при его
использовании. Объект с типизированными значениями записывается как `Map<V>` или
`Record<String, V>`: ключи JSON всегда строки, поэтому первый параметр
фиксирован — `Record<Int, V>` отклоняется диагностикой.

Модули подключаются директивой `use 'path.forma';`. Словари `tokens` и `i18n`
могут быть плоскими или сгруппированными (`tokens color { brand: #fff; }`).
Переиспользуемые наборы свойств объявляются блоком `style name { … }` и
подключаются на узлах через `apply name;`. Именованные точки расширения
объявляются через `slot name(arg: expr) { … }` (разворачиваются в типизированный
`ContentPresenter`) и заполняются в экземпляре через `@name(param) { … }`.
Горячие клавиши объявляются как `shortcut 'Ctrl+S' -> actions.save();`.

```text
Notice {
    title: 'Не удалось сохранить';
    tone: Tone.Danger;
    @leading(tone) {
        Badge { text: String(tone); }
    }
}
```

Опечатка в имени параметра, поля `struct` или варианта `enum`, несовместимый тип
и пропущенное обязательное свойство дают диагностику. События объявляются отдельно:

```text
event open(id: String);
event edited(id: String, value: String);
```

Существующие `events: ['saveRequested'];` и события без аргументов поддерживаются.
Поля Rust ViewModel по-прежнему проверяет Rust-компилятор; их не нужно повторно
объявлять в `.ui`.

Ограничения `#[range(min, max)]` и `#[pattern('шаблон')]` записываются
перед `prop` или полем `struct`:

```text
#[range(1, 280)]
#[pattern('nonempty')]
required prop text: String;
```

`range` ограничивает число, длину строки или длину списка. `pattern` принимает
именованное правило (`nonempty`, `digits`, `alpha`, `alphanumeric`, `email`,
`url`, `hex`) или упрощённый паттерн: литералы, `.`, классы `\d \D \w \W \s \S`
и `[...]` (с отрицанием `[^...]`, диапазонами и экранированием), кванторы `*`,
`+`, `?`, `{n}`, `{n,}`, `{n,m}`, якоря `^` и `$`. Альтернатива `|`
поддерживается только на верхнем уровне и отключается, если в шаблоне есть `[`;
групп `()` нет. Сопоставление ограничено 8192 шагами: слишком сложный шаблон
даёт диагностику с кодом `pattern-too-complex`, а не молчаливое несовпадение.
Литеральные значения по умолчанию и поля литералов `struct` проверяются по
`#[range]` и `#[pattern]` уже на этапе статического анализа.

## Группы свойств, сопоставление с образцом и передача параметров

```text
match props.tone {
    Tone.Danger => {
        background: state.theme.dangerWash;
        borderColor: state.theme.dangerInk;
        icon: 'assets/error.svg';
    }
    Tone.Accent => {
        background: state.theme.accentWash;
        borderColor: state.theme.accentInk;
        icon: 'assets/info.svg';
    }
}
```

`match` работает в трёх позициях: как выражение (`width: match …`), как группа
свойств компонента/узла и как условная разметка дочерних узлов внутри контейнера.
Поддерживаются литералы, варианты `enum`, кортежи `(a, b)`, альтернативы `p1 | p2`,
диапазоны `0..10` / `-5..=5`, сравнения `< 100` и guard-условия `if expr`. Для
конечных замкнутых типов (`Bool`, объявленные `enum`, а также их объединения с
`null` / `T?`) полнота покрытия ветвей проверяется статически (`non-exhaustive-match`).

Приоритет в компоненте: значения по умолчанию → выбранная группа →
явные свойства экземпляра. У дочернего элемента явно записанные свойства
имеют приоритет над группой и `forward`. Несколько выбранных групп не должны
задавать одно и то же свойство.

```text
FieldContent {
    forward props { value, placeholder, fontSize, icon };
    color: props.textColor;
}
```

`forward` передаёт только перечисленные одноимённые параметры из контекста
родительского компонента. Неизвестное имя — ошибка. Переименование записывается
обычным свойством. Автоматической передачи всех свойств нет.

## Изменение узла по ключу

Свойства унаследованного узла можно изменить по `key`, сохранив его тип и детей:

```text
component LargeButton : Button {
    override caption {
        fontSize: 18;
    }
}
```

Для ключа `ContentPresenter` изменяется корень содержимого. Полная замена
содержимого использует двоеточие (`override content: Text { ... };`), а подмена
из внешнего файла — `override content from './Custom.forma' { ... }`.
Правила и примеры — в [описании наследования](vector-ui/COMPONENTS.md).

## Выражения и чистая стандартная библиотека

```text
disabled: state.busy || !state.valid;
width: clamp(state.width - 24, 120, 480);
text: 'Найдено: ${state.count}';
code: '7'.padStart(3, '0');
title: state.rawTitle |> trim |> capitalize;
placeholder: state.customer?.name ?? 'Имя';
```

Обе разновидности кавычек имеют одинаковую семантику. `\${state.count}`
выводит текст `${state.count}` без подстановки. Поддерживаются `\n`, `\r`, `\t`,
экранирование кавычек и обратной косой черты.

Доступны арифметика `+ - * / %` (с сохранением точности типа `Int` для целых
операндов в `+`, `-`, `*`, `%` и унарном `-`), диапазоны `a..b` и `a..=b`,
сравнения, `!`, `&&`, `||`, `??`, конвейер `x |> fn` и UFCS-вызов `x.fn(…)`,
тернарное `condition ? value : alternative`, доступ к полям и индексам,
необязательный доступ `?.`. Логические операторы требуют `Bool` и вычисляют
только нужную ветку. Арифметика принимает конечные числа; деление на ноль — ошибка.
Единицы размеров не участвуют в неявном числовом преобразовании.

Встроенные чистые функции (59 функций, проверяемых по количеству и типам аргументов):

- **Приведение и предикаты:** `String`, `Number`, `Bool`, `len`, `isEmpty`, `coalesce`, `isValid`, `plural`;
- **Математика и интерполяция:** `min`, `max`, `clamp`, `lerp`, `remap`, `abs`, `sign`, `round`, `floor`, `ceil`, `pow`, `sqrt`, `sum`, `avg`;
- **Строки:** `trim`, `upper`, `lower`, `capitalize`, `padStart`, `padEnd`, `truncate`, `repeat`, `startsWith`, `endsWith`, `contains`, `includes`, `indexOf`, `replace`, `split`, `slice`, `join`, `reverse`, `formatNumber`;
- **Списки и словари:** `range`, `count`, `any`, `all`, `first`, `last`, `take`, `skip`, `compact`, `flatten`, `chunk`, `zip`, `pluck`, `find`, `groupBy`, `sortBy`, `unique`, `keys`, `values`, `entries`, `fromEntries`, `merge`, `pick`, `omit`;
- **Цвет и контраст (WCAG):** `alpha`, `mix`, `lighten`, `darken`, `saturate`, `desaturate`, `invert`, `contrast`.

Для вычислений с побочными эффектами используются обработчики в Rust.

## Условия и списки

```text
Column {
    gap: 8;

    if state.loading {
        Skeleton {}
    } else {
        Text { text: 'Готово'; }
    }

    for item, idx in state.items where !item.archived key item.id {
        Button {
            text: '${idx + 1}. ${item.title}';
            clicked -> events.open(item.id);
        }
    } empty {
        Text { text: 'Список пуст'; }
    }
}
```

`if` создаёт выбранную ветку. `for item, idx in collection where cond key expr`
поддерживает необязательный индекс `idx` (`Int`), фильтр `where` (`Bool`) и ветку
`empty` для пустого результата; для числовых диапазонов `0..n` и `range(0, n)`
ключ по умолчанию равен элементу. Ключ — строка или конечное число, уникальное
в пределах повторителя. Ключи разных типов различаются. Идентичность включает
место повторителя и ключ, поэтому перестановка коллекции сохраняет фокус и
состояние оставшихся контролов. Удаление сфокусированного элемента снимает фокус.
Повторные ключи — ошибка.

Вложенные повторители используют разные имена переменных. В Rust коллекция
может быть `Property<Vec<Row>>`, где `Row: Clone + PartialEq`. Генератор
извлекает используемые поля без ручного преобразования всей модели в словарь.

```text
for item in state.items key item.id {
    TextField {
        value <-> item.title;
        changed -> events.edited(item.id, item.title);
    }
}
```

Запись в модель происходит перед событием `changed`; аргументы события
соответствуют актуальной строке и новому значению. Подробности подписок,
перепривязки контекста и Rust API — в [BINDINGS.md](vector-ui/BINDINGS.md).

## Раскладка

- `Row`: последовательность по горизонтали.
- `Column`: последовательность по вертикали.
- `Grid`: сетка с автоматическим размещением свободных элементов.
- `Stack`: наложение элементов.

```text
Row {
    gap: 8;
    if props.showIcon {
        Icon { width: 16; height: 16; source: props.icon; }
    }
    FieldValue { width: *; minWidth: 80; }
}

Grid {
    columns: [160, *];
    gap: 8;
    Button { text: 'Первый'; }
    Button { text: 'Второй'; }
    Button { text: 'Следующая строка'; }
}
```

В `Row`/`Column` основной размер по умолчанию определяется содержимым;
`*` и `2*` делят оставшееся место. Поперечный размер без указания растягивается,
явно заданный размер центрируется. Учитываются `minWidth`, `maxWidth`,
`minHeight`, `maxHeight`; минимум не может превышать максимум.
`auto`, `content` и `-` используют размер по содержимому, включая метрики текста.

В `Grid` явные `cell`/`row`/`column` занимают свои ячейки; остальные элементы
заполняют свободные места по строкам. При необходимости добавляются строки
по содержимому. Поддерживаются `row.span` и `column.span`.

Прежний `Frame` сохраняет совместимость: корневой Frame без сетки размещает
контролы вертикально, Frame внутри визуального компонента накладывает детей.
Явные `Row`/`Column`/`Grid`/`Stack` позволяют выразить нужный режим без этого
контекстного различия. Векторная сцена по-прежнему ограничена 256 контролами.
Обрезка корня и внутренних визуальных частей поддерживается; вложенные
контейнеры сцены с отдельным `clip` требуют дальнейшей поддержки renderer.

`Modal { key; x; y; width; height; backdrop; dismiss; open; dismissed -> actions.name(); … }`
объявляет модальный слой после контролов сцены. Без `x`/`y` слой центрируется
в корне; дети размещаются внутри его поверхности как в `Column`, а в свободной
раскладке сохраняют свои `x`/`y` относительно слоя. Открытый слой получает весь
ввод: контролы под backdrop недоступны, Tab ходит по кругу, Escape и нажатие вне
поверхности вызывают `dismissed`. Подробности и API runtime описаны в
[README векторного среза](vector-ui/README.md).

## Диагностики

Ошибки разбора, проверки контракта и вычисления выражений возвращаются как
структурированная диагностика `{code, message, span}` (тип `Diagnostic`
в `crates/tokenizer-forma`). `code` — стабильный идентификатор в kebab-case,
например `syntax-error`, `invalid-call`, `invalid-operator`, `unknown-state`,
`expected-number`, `pattern-too-complex`; ошибки хоста и резолверов получают
общий код `host-error`. `message` — человекочитаемый текст; `span` — позиция
в UTF-16 единицах (та же шкала, что у offsets парсера, для CodeMirror),
отсутствует, когда позиция неизвестна. `Display` выводит только `message`,
поэтому потребители прежних строковых ошибок работают без изменений.
Через wasm-мост ошибка приходит как JS `Error` с тем же `message` и
дополнительными полями `code` и `span`.

## Линтер и статическая проверка

Операция `lint` wasm-моста (`lint_document`) и JS-функция `lintDocument(source)`
возвращают массив записей `{code, message, from, to, …}`. Все диапазоны `from`/`to`
выражаются в UTF-16 единицах (включая `unused-style` и `unreachable-branch`).
Предупреждения:

- `unused-let`, `unused-prop`, `unused-state`, `unused-style` — объявление
  не используется ни в одном выражении документа;
- `unused-token`, `unused-i18n-key` — ключ словаря `tokens`/`i18n` нигде
  не упоминается; в файле общего модуля без тела компонента эти проверки
  отключены, так как ключи образуют его публичный API;
- `duplicate-shortcut` — повторная комбинация клавиш (сравнение без учёта
  регистра), вместо `name` возвращается поле `key`;
- `unreachable-branch` — ветка `match` после шаблона `_` без охраны.

Статическая проверка контрактов и выражений добавляет записи с `severity: "error"`,
строками `expected`/`actual` и точным UTF-16 диапазоном виновного узла:

- `incompatible-binding` — значение свойства, default, поле литерала `struct`,
  двусторонняя привязка, коллекция или ключ `for` несовместимы с ожидаемым типом;
- `incompatible-argument` — неверный тип аргумента чистой функции, события или action;
- `invalid-argument-count` — неверное число аргументов чистой функции, объявленного
  `event` или типизированного `action`;
- `unknown-function` — вызов неизвестной чистой функции;
- `incompatible-condition` — условие `if`, `where`, guard или тернарного оператора
  не является `Bool`;
- `incompatible-operand` — операнд не подходит унарному, бинарному оператору или
  строковой интерполяции;
- `incompatible-match-pattern` — паттерн (включая диапазоны и кортежи) не подходит
  известному типу subject;
- `incompatible-match-branch` — результат ветви не подходит ожидаемому типу `match`;
- `non-exhaustive-match` — `match` по конечному замкнутому типу (`Bool`, `enum`, `T?`)
  не покрывает все варианты без guard и не имеет ветки `_`;
- `unknown-variant` — обращение к несуществующему варианту объявленного `enum`;
- `unknown-field` — обращение к несуществующему полю объявленного `struct` (в литерале,
  цепочке `.field` или строковом ключе `pluck`/`groupBy`/`sortBy`/`unique`/`find`/`pick`/`omit`);
- `unknown-style` — `apply` ссылается на необъявленный стиль в самодостаточном
  компоненте без `use` и наследования;
- `invalid-range` — константный диапазон с `min > max` в `clamp` или вырожденный
  входной интервал `inMin == inMax` в `remap`;
- `out-of-range`, `pattern-mismatch`, `pattern-too-complex` — нарушение атрибутов
  `#[range]` и `#[pattern]` на статических литералах.

`lintDocument(source, context)` принимает необязательный контекст хоста:
`{properties, components, actions}`. `properties` и `components` имеют вид
`узел → свойство → строка типа`, `actions` — `имя → массив строк типов аргументов`.
Эти записи накладываются на версионируемый встроенный каталог `tokenizer-forma`;
`components` задаёт контракты дочерних компонентов. Rust API называется
`lint_document_with_context`, тип контекста — `TypeContext`. Некорректный JSON
контекста завершает WASM-операцию ошибкой `invalid-type-context`.

Ошибка выводится только при доказанной несовместимости. `Unknown`, `Any`,
отсутствующая host-схема и неизвестный динамический путь подавляют диагностику;
legacy default внешне переопределяемого prop не задаёт тип его чтения. Любая
доказанно несовместимая известная альтернатива фактического union является
ошибкой, если среди альтернатив нет `Unknown`/`Any`. Проверка не меняет AST,
`validate_contract` или runtime-вычисление.

Значения по умолчанию состояний считаются реальными выражениями и удерживают
свои ссылки; резервное значение prop удерживает состояния, стили, токены
и ключи i18n, но не другие свойства.

## Примеры

Запускаемый [Rust-пример со списком задач](vector-ui/examples/language-app/README.md)
показывает типизированные свойства и события, группы, `forward`, выражения,
редактирование и перестановку строк, `Row` и `Grid`:

```sh
cargo run --locked --manifest-path vector-ui/examples/language-app/Cargo.toml
```

Обычные `Text` и `TextInput` работают в сцене без файлов компонентов.
При наличии `components/Text.ui` или `components/TextInput.ui` используется
определение проекта. Внутри визуального дерева они остаются примитивами.

В библиотеке [Alert](vector-ui/controls/components/Alert.ui) использует
групповой `match`, [TextField](vector-ui/controls/components/TextField.ui) —
`forward`, [FieldContent](vector-ui/controls/components/FieldContent.ui) —
`Row` с условной иконкой без дублирования поля ввода.
