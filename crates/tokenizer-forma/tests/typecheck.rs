use serde_json::{Value, json};
use themoretheless_tokenizer_forma::lint_document_with_context;
use themoretheless_tokenizer_forma::typecheck::{TypeContext, analyze_source};

fn report(source: &str) -> themoretheless_tokenizer_forma::typecheck::TypecheckReport {
    analyze_source(source, &TypeContext::default()).unwrap()
}

fn incompatible(source: &str, code: &str) -> Value {
    let diagnostics = report(source).diagnostics;
    let matches = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "{source}\n{diagnostics:#?}");
    matches[0].clone()
}

fn utf16_range(source: &str, needle: &str) -> (usize, usize) {
    let byte = source.find(needle).unwrap();
    let from = source[..byte].encode_utf16().count();
    (from, from + needle.encode_utf16().count())
}

fn assert_range(source: &str, code: &str, needle: &str) -> Value {
    let diagnostic = incompatible(source, code);
    let (from, to) = utf16_range(source, needle);
    assert_eq!(diagnostic["from"], from, "{diagnostic:#?}");
    assert_eq!(diagnostic["to"], to, "{diagnostic:#?}");
    assert_eq!(diagnostic["severity"], "error");
    assert!(diagnostic["expected"].is_string());
    assert!(diagnostic["actual"].is_string());
    diagnostic
}

#[test]
fn bindings_arguments_conditions_operators_and_units_are_checked() {
    assert_range(
        "component A { state name: String = '😀'; Text { width: state.name; } }",
        "incompatible-binding",
        "state.name",
    );
    assert_range(
        "component A { state n: Number = 1; Text { text: trim(state.n); } }",
        "incompatible-argument",
        "state.n",
    );
    assert_range(
        "component A { state name: String = 'x'; if state.name { Text {} } }",
        "incompatible-condition",
        "state.name",
    );
    assert_range(
        "component A { state name: String = 'x'; Text { width: state.name + 1; } }",
        "incompatible-operand",
        "state.name + 1",
    );
    assert_range(
        "component A { Text { transitionDuration: 1px; } }",
        "incompatible-binding",
        "1px",
    );
    assert_range(
        "component A { state n: Number = 1; TextField { value <-> state.n; } }",
        "incompatible-binding",
        "value <-> state.n;",
    );
}

#[test]
fn match_patterns_branches_and_guards_have_expression_ranges() {
    assert_range(
        "component A { state n: Number = 1; Text { text: match state.n { '😀' => 'x'; _ => 'y'; }; } }",
        "incompatible-match-pattern",
        "'😀'",
    );
    assert_range(
        "component A { state flag: Bool = true; Text { width: match state.flag { true => 1px; _ => 'bad'; }; } }",
        "incompatible-match-branch",
        "'bad'",
    );
    let source = "component A { state n: Number = 1; Text { text: match state.n { 1 if state.n => 'x'; _ => 'y'; }; } }";
    let diagnostic = incompatible(source, "incompatible-condition");
    let byte = source.rfind("state.n").unwrap();
    let from = source[..byte].encode_utf16().count();
    assert_eq!(diagnostic["from"], from);
    assert_eq!(diagnostic["to"], from + "state.n".len());
}

#[test]
fn nested_interpolation_offsets_are_absolute_utf16() {
    let source = "component A { state n: Number = 1; Text { text: '😀 ${trim(state.n)}'; } }";
    let diagnostic = incompatible(source, "incompatible-argument");
    let byte = source.rfind("state.n").unwrap();
    let from = source[..byte].encode_utf16().count();
    assert_eq!(diagnostic["from"], from);
    assert_eq!(diagnostic["to"], from + "state.n".encode_utf16().count());
}

#[test]
fn unknown_any_and_missing_schemas_suppress_diagnostics() {
    for source in [
        "component A { state value: Unknown = external.value; Text { width: state.value; } }",
        "component A { state value: String | Any = 'x'; Text { width: state.value; } }",
        "component A { state value: Any | String = 'x'; Text { width: state.value; } }",
        "component A { state name: String = 'x'; Text { width: state.name; } }",
        "component A { state values: List<Number> = []; Text { width: sum(state.values); } }",
        "component A { text: 1; Text { text: props.text; } }",
        "component A { Text { color: invert(#fff); } }",
    ] {
        let context = if source.contains("width: state.name") {
            TypeContext::empty()
        } else {
            TypeContext::default()
        };
        let diagnostics = analyze_source(source, &context).unwrap().diagnostics;
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
    }
}

#[test]
fn known_union_members_still_prove_incompatibility() {
    assert_range(
        "component A { state value: String | Int = 1; Text { width: state.value; } }",
        "incompatible-binding",
        "state.value",
    );
    let source = "component A { state n: Number = 1; Text { width: state.n; } }";
    assert!(report(source).diagnostics.is_empty());
}

#[test]
fn host_context_overrides_builtins_and_types_child_components_and_actions() {
    let context = TypeContext::from_json(&json!({
        "properties": {"TextField": {"value": "Number"}},
        "components": {"Child": {"title": "String"}},
        "actions": {"save": ["String"]}
    }))
    .unwrap();
    let good = "component A { state n: Number = 1; TextField { value <-> state.n; } Child { title: 'ok'; clicked -> actions.save('ok'); } }";
    assert!(
        analyze_source(good, &context)
            .unwrap()
            .diagnostics
            .is_empty()
    );
    let bad = "component A { state n: Number = 1; Child { title: state.n; clicked -> actions.save(state.n); } }";
    let diagnostics = analyze_source(bad, &context).unwrap().diagnostics;
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "incompatible-binding")
            .count(),
        1
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "incompatible-argument")
            .count(),
        1
    );
}

#[test]
fn public_lint_adds_only_additive_diagnostics() {
    let source = "component A { state name: String = 'x'; Text { width: state.name; } }";
    let diagnostics = lint_document_with_context(source, &TypeContext::default()).unwrap();
    let diagnostic = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "incompatible-binding")
        .unwrap();
    assert_eq!(diagnostic["severity"], "error");
    assert!(TypeContext::from_json(&json!([])).is_err());
}

#[test]
fn corpus_has_no_unreviewed_type_diagnostics_and_has_real_coverage() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/forma-corpus.json")).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 87);
    let context = TypeContext::default();
    let mut total = 0;
    let mut typed = 0;
    let mut diagnostics = Vec::new();
    for case in cases.as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let report = analyze_source(case["source"].as_str().unwrap(), &context).unwrap();
        total += report.expressions_total;
        typed += report.expressions_typed;
        for diagnostic in report.diagnostics {
            eprintln!(
                "CORPUS {file} {} {}..{} expected={} actual={}",
                diagnostic["code"],
                diagnostic["from"],
                diagnostic["to"],
                diagnostic["expected"],
                diagnostic["actual"]
            );
            diagnostics.push((file, diagnostic));
        }
    }
    eprintln!("CORPUS COVERAGE typed={typed} total={total}");
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(
        (typed, total),
        (1775, 2280),
        "Reviewed corpus coverage baseline"
    );
}

#[test]
fn stdlib_collection_and_record_calls_preserve_element_types() {
    let valid = r#"
        struct Item {
            price: Number;
            label: String;
        }
        component Cart {
            prop nums: List<Number?> = [1, null, 2];
            prop grid: List<List<Number>> = [[1, 2], [3]];
            prop items: List<Item> = [];
            prop scores: Record<String, Number> = {};
            prop bonus: Record<String, Int> = {};
            prop firstItem: Item = { price: 10, label: 'ok' };
            Text {
                width: sum(compact(props.nums))
                    + sum(flatten(props.grid))
                    + sum(pluck(props.items, 'price'))
                    + sum(values(pick(props.scores, ['a'])))
                    + sum(values(omit(props.scores, ['b'])))
                    + sum(values(merge(props.scores, props.bonus)))
                    + sum(values(pick(props.firstItem, ['price'])));
            }
        }
    "#;
    let valid_report = report(valid);
    assert!(
        valid_report.diagnostics.is_empty(),
        "{:#?}",
        valid_report.diagnostics
    );

    let d1 = assert_range(
        "component A { prop tags: List<String?> = ['a', null]; Text { width: sum(compact(props.tags)); } }",
        "incompatible-argument",
        "compact(props.tags)",
    );
    assert_eq!(d1["actual"], "List<String>");

    let d2 = assert_range(
        "component A { prop rows: List<List<String>> = [['a']]; Text { width: sum(flatten(props.rows)); } }",
        "incompatible-argument",
        "flatten(props.rows)",
    );
    assert_eq!(d2["actual"], "List<String>");

    let d3 = assert_range(
        "struct Item { label: String; } component A { prop items: List<Item> = []; Text { width: sum(pluck(props.items, 'label')); } }",
        "incompatible-argument",
        "pluck(props.items, 'label')",
    );
    assert_eq!(d3["actual"], "List<String>");

    let d4 = assert_range(
        "struct Item { label: String; } component A { prop item: Item = { label: 'x' }; Text { width: sum(values(pick(props.item, ['label']))); } }",
        "incompatible-argument",
        "values(pick(props.item, ['label']))",
    );
    assert_eq!(d4["actual"], "List<String>");

    // Structural checking on struct literals (including nested in List<Item>)
    // and match expressions inside ternary conditions:
    let nested_ok = r#"
        struct Item { price: Number; label: String; tag: String?; }
        component A {
            state x: Int = 1;
            prop items: List<Item> = [{ price: 5, label: 'ok' }];
            Text {
                width: (match state.x { 1 => true; _ => false; }) ? 10px : 20px;
            }
        }
    "#;
    assert!(report(nested_ok).diagnostics.is_empty());

    assert_range(
        "struct Item { price: Number; label: String; } component A { prop items: List<Item> = [{ price: 'bad', label: 'ok' }]; }",
        "incompatible-binding",
        "'bad'",
    );
    assert_range(
        "struct Item { price: Number; label: String; } component A { prop item: Item = { price: 10 }; }",
        "incompatible-binding",
        "{ price: 10 }",
    );
}

#[test]
fn span_soundness_bare_props_sibling_duplicates_ufcs_pipe_and_match_blocks() {
    let bare_call = "component A { state val: Number = 1; Text { text: trim(val); } }";
    let d_bare = incompatible(bare_call, "incompatible-argument");
    let at_val = bare_call.rfind("val").unwrap();
    assert_eq!(d_bare["from"], bare_call[..at_val].encode_utf16().count());
    assert_eq!(d_bare["to"], bare_call[..at_val].encode_utf16().count() + 3);

    assert_range(
        "component A { state n: Number = 1; Text { disabled: !n; } }",
        "incompatible-operand",
        "!n",
    );
    assert_range(
        "component A { Text { width: 'bad' - 1; } }",
        "incompatible-operand",
        "'bad'",
    );
    assert_range(
        "component A { Text { disabled: 'bad' && true; } }",
        "incompatible-operand",
        "'bad'",
    );
    assert_range(
        "component A { Text { width: 'bad' ? 10px : 20px; } }",
        "incompatible-condition",
        "'bad'",
    );

    let ufcs_call = "component A { state val: Number = 1; Text { text: val.trim(); } }";
    let d_ufcs = incompatible(ufcs_call, "incompatible-argument");
    let at_ufcs = ufcs_call.rfind("val").unwrap();
    assert_eq!(d_ufcs["from"], ufcs_call[..at_ufcs].encode_utf16().count());
    assert_eq!(
        d_ufcs["to"],
        ufcs_call[..at_ufcs].encode_utf16().count() + 3
    );

    let pipe_call = "component A { state val: Number = 1; Text { text: val |> trim; } }";
    let d_pipe = incompatible(pipe_call, "incompatible-argument");
    let at_pipe = pipe_call.rfind("val").unwrap();
    assert_eq!(d_pipe["from"], pipe_call[..at_pipe].encode_utf16().count());
    assert_eq!(
        d_pipe["to"],
        pipe_call[..at_pipe].encode_utf16().count() + 3
    );

    let dup_call = "component A { Text { text: repeat('5', '5'); } }";
    let diag = incompatible(dup_call, "incompatible-argument");
    let second = dup_call.rfind("'5'").unwrap();
    let from = dup_call[..second].encode_utf16().count();
    assert_eq!(diag["from"], from);
    assert_eq!(diag["to"], from + "'5'".len());

    let dup_interp = "component A { prop items: List<Int> = [1]; Text { text: '${1} ${items}'; } }";
    let d_interp = incompatible(dup_interp, "incompatible-operand");
    let at_items = dup_interp.rfind("items").unwrap();
    assert_eq!(
        d_interp["from"],
        dup_interp[..at_items].encode_utf16().count()
    );
    assert_eq!(
        d_interp["to"],
        dup_interp[..at_items].encode_utf16().count() + "items".len()
    );

    assert_range(
        "component A { state flag: Bool = true; match state.flag { 'bad' => { width: 10px; } _ => { width: 20px; } } }",
        "incompatible-match-pattern",
        "'bad'",
    );
    assert_range(
        "component A { state flag: Bool = true; Row { match state.flag { 'bad' => Text {} _ => Text {} } } }",
        "incompatible-match-pattern",
        "'bad'",
    );
    assert_range(
        "component A { state s: String = 'a'; Text { text: match state.s { 1..=5 => 'num'; _ => 'other'; }; } }",
        "incompatible-match-pattern",
        "1..=5",
    );
    assert_range(
        "component A { state a: Int = 1; state b: Int = 2; Text { text: match (state.a, state.b) { (1, 'bad') => 'x'; _ => 'y'; }; } }",
        "incompatible-match-pattern",
        "'bad'",
    );
}

#[test]
fn exhaustiveness_enum_variants_struct_fields_arities_int_precision_and_constraints() {
    assert_range(
        "enum Tone { Accent, Danger, Muted } component A { prop tone: Tone = Tone.Accent; Text { text: match props.tone { Tone.Accent => 'a'; Tone.Danger => 'd'; }; } }",
        "non-exhaustive-match",
        "match props.tone { Tone.Accent => 'a'; Tone.Danger => 'd'; }",
    );
    assert_range(
        "component A { state flag: Bool? = null; Row { match state.flag { true => Text {} false => Text {} } } }",
        "non-exhaustive-match",
        "match state.flag { true => Text {} false => Text {} }",
    );
    let exhaustive_enum = "enum Tone { Accent, Danger } component A { prop tone: Tone = Tone.Accent; Text { text: match props.tone { Tone.Accent | Tone.Danger => 'ok'; }; } }";
    assert!(report(exhaustive_enum).diagnostics.is_empty());
    assert_range(
        "component A { state flag: Bool = true; Text { text: match state.flag { true if state.flag => 'a'; false => 'b'; }; } }",
        "non-exhaustive-match",
        "match state.flag { true if state.flag => 'a'; false => 'b'; }",
    );

    assert_range(
        "enum Tone { Accent, Danger } component A { prop tone: Tone = Tone.Dnger; }",
        "unknown-variant",
        "Tone.Dnger",
    );

    let extra_field = "struct Item { price: Number; label: String; } component A { prop item: Item = { price: 10, label: 'ok', extra: 1 }; }";
    let d_extra = incompatible(extra_field, "unknown-field");
    let at_extra_val = extra_field.rfind('1').unwrap();
    assert_eq!(
        d_extra["from"],
        extra_field[..at_extra_val].encode_utf16().count()
    );
    assert_eq!(
        d_extra["to"],
        extra_field[..at_extra_val].encode_utf16().count() + 1
    );

    assert_range(
        "struct Item { price: Number; label: String; } component A { prop item: Item = { price: 10, label: 'ok' }; Text { width: props.item.pirce; } }",
        "unknown-field",
        "props.item.pirce",
    );
    assert_range(
        "struct Item { price: Number; label: String; } component A { prop items: List<Item> = []; Text { width: sum(pluck(props.items, 'pirce')); } }",
        "unknown-field",
        "'pirce'",
    );
    assert_range(
        "struct Item { id: Int; label: String; } component A { prop items: List<Item> = []; let found = find(props.items, 'id', 'not-an-int'); }",
        "incompatible-argument",
        "'not-an-int'",
    );

    assert_range(
        "component A { Text { width: clamp(10, 20); } }",
        "invalid-argument-count",
        "clamp(10, 20)",
    );
    assert_range(
        "component A { Text { width: min(); } }",
        "invalid-argument-count",
        "min()",
    );
    assert_range(
        "component A { event picked(index: Int); Button { clicked -> events.picked(1, 2); } }",
        "invalid-argument-count",
        "clicked -> events.picked(1, 2);",
    );
    assert_range(
        "component A { state x: Number = 5; Text { width: clamp(state.x, 100, 20); } }",
        "invalid-range",
        "clamp(state.x, 100, 20)",
    );

    let int_ok = r#"
        component Pager {
            state page: Int = 0;
            event selected(idx: Int);
            Row {
                width: 50vmin;
                height: 80vmax;
                for n in 0..10 key n {
                    Button {
                        clicked -> events.selected(clamp(abs(state.page + n - 1), 0, round(9.4)));
                    }
                }
            }
        }
    "#;
    assert!(
        report(int_ok).diagnostics.is_empty(),
        "{:#?}",
        report(int_ok).diagnostics
    );

    assert_range(
        "component A { #[range(1, 10)] prop count: Int = 25; }",
        "out-of-range",
        "25",
    );
    assert_range(
        "component A { #[pattern('digits')] prop code: String = 'abc'; }",
        "pattern-mismatch",
        "'abc'",
    );
    assert_range(
        "struct Pin { #[pattern('^[0-9]{4}$')] code: String; } component A { prop pin: Pin = { code: '12ab' }; }",
        "pattern-mismatch",
        "'12ab'",
    );

    assert_range(
        "component A { style card { width: 'bad'; } }",
        "incompatible-binding",
        "'bad'",
    );
    let ov_src = "component A : Base { state val: Number = 1; override content: trim(val); }";
    let d_ov = incompatible(ov_src, "incompatible-argument");
    let at_ov = ov_src.rfind("val").unwrap();
    assert_eq!(d_ov["from"], ov_src[..at_ov].encode_utf16().count());
    assert_eq!(d_ov["to"], ov_src[..at_ov].encode_utf16().count() + 3);
}

#[test]
fn flow_sensitive_type_narrowing_across_control_flow_and_short_circuit_ops() {
    let valid = r#"
        enum Tone { Accent, Muted }
        struct Badge { label: String; count: Int; }
        struct User { name: String; badge: Badge?; }
        component NarrowDemo {
            prop subtitle: String? = null;
            state label: String? = null;
            state code: String? = null;
            state score: Number? = null;
            state user: User? = null;
            state mixed: Tone | String = Tone.Accent;
            state a: Int? = null;
            state b: Int? = null;
            state users: List<User?> = [];

            if state.label != null {
                Text { text: trim(state.label); }
            } else {
                Text { text: 'empty'; }
            }

            if subtitle == null {
                Text { text: 'none'; }
            } else {
                Text { text: upper(subtitle); }
            }

            if !(state.a == null || state.b == null) {
                Text { width: state.a + state.b; }
            }

            if !isEmpty(state.label) {
                Text { text: lower(state.label); }
            }

            if isValid(state.code, 'digits') && isValid(state.score, 0, 100) {
                Text {
                    text: padStart(state.code, 4, '0');
                    width: state.score + 10;
                }
            }

            if state.user != null && state.user.badge != null {
                Text {
                    text: trim(state.user.badge.label);
                    width: state.user.badge.count + 1;
                }
            }

            Text {
                disabled: state.label == null || trim(state.label) == '';
                text: props.subtitle != null ? trim(props.subtitle) : 'fallback';
            }

            Text {
                text: match state.label {
                    null => 'none';
                    _ => trim(state.label);
                };
                width: match (state.a, state.b) {
                    (null, _) => 0px;
                    (_, null) => 0px;
                    _ => state.a + state.b;
                };
            }

            Text {
                text: match state.mixed {
                    Tone.Accent | Tone.Muted => 'tone';
                    _ => trim(state.mixed);
                };
            }

            Text {
                text: match state.label {
                    _ if state.label != null => capitalize(state.label);
                    _ => 'fallback';
                };
            }

            Row {
                match state.label {
                    null => { show: false; }
                    _ => { show: trim(state.label) != ''; }
                }
                match state.label {
                    null => Text { text: 'empty'; }
                    _ => Text { text: trim(state.label); }
                }
            }

            for item in state.users where item != null && item.badge != null key item.name {
                Text {
                    text: trim(item.badge.label);
                    width: item.badge.count;
                }
            }
        }
    "#;
    let valid_report = report(valid);
    assert!(
        valid_report.diagnostics.is_empty(),
        "{:#?}",
        valid_report.diagnostics
    );

    // Guarded `null` branch does NOT exhaust `null` for subsequent branches:
    let guarded_null = "component A { state flag: Bool = true; state label: String? = null; Text { text: match state.label { null if state.flag => 'a'; _ => trim(state.label); }; } }";
    let d_guard = incompatible(guarded_null, "incompatible-argument");
    let at_guard = guarded_null.rfind("state.label").unwrap();
    assert_eq!(
        d_guard["from"],
        guarded_null[..at_guard].encode_utf16().count()
    );
    assert_eq!(d_guard["actual"], "String | Null");

    // Narrowing `state.user` from `User?` to `User` exposes real field types and catches mismatches:
    let narrowed_err = "struct User { age: Int; } component A { state user: User? = null; if state.user != null { Text { text: trim(state.user.age); } } }";
    let diag = incompatible(narrowed_err, "incompatible-argument");
    let at = narrowed_err.rfind("state.user.age").unwrap();
    assert_eq!(diag["from"], narrowed_err[..at].encode_utf16().count());
    assert_eq!(diag["actual"], "Int");

    // In the `else` branch of `if state.label != null`, `state.label` is narrowed to `Null`:
    let else_null = "component A { state label: String? = null; if state.label != null { Text { text: state.label; } } else { Text { text: trim(state.label); } } }";
    let d_else = incompatible(else_null, "incompatible-argument");
    let at_else = else_null.rfind("state.label").unwrap();
    assert_eq!(d_else["from"], else_null[..at_else].encode_utf16().count());
    assert_eq!(d_else["actual"], "Null");

    // `let` shadowing inside a child container resets narrowing for local names:
    let shadowed = "component A { state raw: String? = null; Frame { let x = state.raw; if x != null { Frame { let x = state.raw; Text { text: trim(x); } } } } }";
    let d_shadow = incompatible(shadowed, "incompatible-argument");
    let at_x = shadowed.rfind("x)").unwrap();
    assert_eq!(d_shadow["from"], shadowed[..at_x].encode_utf16().count());
}

#[test]
fn cross_document_modules_inheritance_and_required_props_work() {
    let theme_module = r#"
        enum Tone { Accent, Danger, Muted }
        struct PriceTag {
            #[range(1, 100)]
            amount: Number;
            #[pattern('^[A-Z]{3}$')]
            currency: String;
        }
        struct Product {
            title: String;
            tag: PriceTag;
        }
        tokens {
            brand: #ff0088;
            spacing.md: 12px;
        }
        i18n en {
            greeting: 'Hello';
        }
        style card {
            width: 120px;
        }
    "#;
    let base_card = r#"
        component BaseCard {
            required prop title: String;
            #[range(1, 5)]
            prop elevation: Int = 1;
            prop tone: Tone = Tone.Accent;
            event selected(id: Int);
            style elevated {
                height: 48px;
            }
            Surface {
                Text { text: props.title; }
            }
        }
    "#;
    let primary_card = r#"
        component PrimaryCard : BaseCard {
            title: 'Default Primary';
            Surface {
                apply elevated;
                Text { text: props.title; }
            }
        }
    "#;

    let context = TypeContext::from_json(&json!({
        "modules": {
            "./theme.forma": theme_module,
            "./PrimaryCard.forma": primary_card,
            "./BaseCard.forma": base_card
        }
    }))
    .unwrap();

    let valid_page = r#"
        use './theme.forma';
        use './BaseCard.forma';
        use './PrimaryCard.forma';
        component StorePage {
            prop product: Product = {
                title: 'Tea',
                tag: { amount: 25, currency: 'USD' }
            };
            Surface {
                apply card;
                width: tokens.spacing.md;
                color: tokens.brand;
                BaseCard {
                    title: i18n.greeting;
                    elevation: 3;
                    tone: match props.product.tag.currency {
                        'USD' => Tone.Accent;
                        _ => Tone.Muted;
                    };
                }
                PrimaryCard {
                    elevation: 2;
                }
            }
        }
    "#;
    let ok_lints = lint_document_with_context(valid_page, &context).unwrap();
    assert_eq!(ok_lints, json!([]), "{ok_lints:#?}");

    // Cross-file enum unknown-variant & non-exhaustive-match:
    let bad_enum = r#"
        use './theme.forma';
        component BadEnum {
            prop tone: Tone = Tone.Bogus;
            Text {
                text: match props.tone {
                    Tone.Accent => 'a';
                    Tone.Danger => 'd';
                };
            }
        }
    "#;
    let enum_diags = analyze_source(bad_enum, &context).unwrap().diagnostics;
    assert!(enum_diags.iter().any(|d| d["code"] == "unknown-variant"));
    assert!(
        enum_diags
            .iter()
            .any(|d| d["code"] == "non-exhaustive-match" && d["expected"] == "Tone.Muted")
    );

    // Cross-file nested struct constraint & token type mismatch:
    let bad_struct_and_token = r#"
        use './theme.forma';
        component BadProduct {
            prop product: Product = {
                title: 'Tea',
                tag: { amount: 250, currency: 'usd' }
            };
            Text {
                width: tokens.brand;
                text: props.product.title;
            }
        }
    "#;
    let st_diags = analyze_source(bad_struct_and_token, &context)
        .unwrap()
        .diagnostics;
    assert!(st_diags.iter().any(|d| d["code"] == "out-of-range"));
    assert!(st_diags.iter().any(|d| d["code"] == "pattern-mismatch"));
    assert!(
        st_diags
            .iter()
            .any(|d| d["code"] == "incompatible-binding" && d["actual"] == "Color")
    );

    // Missing required prop & child static constraint check:
    let bad_child = r#"
        use './BaseCard.forma';
        component Parent {
            BaseCard {
                elevation: 99;
            }
        }
    "#;
    let child_diags = analyze_source(bad_child, &context).unwrap().diagnostics;
    assert!(
        child_diags
            .iter()
            .any(|d| d["code"] == "missing-required-prop" && d["expected"] == "title: String"),
        "{child_diags:#?}"
    );
    assert!(child_diags.iter().any(|d| d["code"] == "out-of-range"));

    // Unknown style is caught when all `use` modules are registered, but suppressed without context:
    let bad_style = r#"
        use './theme.forma';
        component Styled {
            Surface { apply nonexistent; }
        }
    "#;
    let style_with_ctx = lint_document_with_context(bad_style, &context).unwrap();
    assert!(
        style_with_ctx
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "unknown-style" && d["actual"] == "nonexistent")
    );
    let style_without_ctx = lint_document_with_context(bad_style, &TypeContext::default()).unwrap();
    assert_eq!(style_without_ctx, json!([]));

    // Derived component inherits base props, constraints, events, and styles:
    let bad_derived = r#"
        component CustomCard : BaseCard {
            elevation: 99;
            Button {
                apply missingStyle;
                clicked -> events.selected('not-an-int');
            }
        }
    "#;
    let derived_lints = lint_document_with_context(bad_derived, &context).unwrap();
    let arr = derived_lints.as_array().unwrap();
    assert!(arr.iter().any(|d| d["code"] == "out-of-range"), "{arr:#?}");
    assert!(
        arr.iter().any(|d| d["code"] == "incompatible-argument"),
        "{arr:#?}"
    );
    assert!(arr.iter().any(|d| d["code"] == "unknown-style"), "{arr:#?}");

    // Design override files do not require `required prop`s, while still enforcing prop constraints:
    let design_override = r#"
        design Compact {
            BaseCard {
                key: 'hero';
                elevation: 99;
            }
        }
    "#;
    let design_diags = analyze_source(design_override, &context)
        .unwrap()
        .diagnostics;
    assert!(
        !design_diags
            .iter()
            .any(|d| d["code"] == "missing-required-prop")
    );
    assert!(design_diags.iter().any(|d| d["code"] == "out-of-range"));
}
