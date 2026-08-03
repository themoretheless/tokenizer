use std::panic::{AssertUnwindSafe, catch_unwind};

use themoretheless_tokenizer::json::{lex, parse, tokenize};

#[derive(Clone, Copy)]
struct DeterministicRng(u64);

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, upper: usize) -> usize {
        (self.next() as usize) % upper
    }
}

const TEXT_CHARACTERS: &[char] = &[
    '{', '}', '[', ']', ':', ',', '"', '\\', '/', '-', '+', '.', '0', '1', '7', '9', 'e', 'E', 't',
    'r', 'u', 'f', 'a', 'l', 's', 'n', ' ', '\t', '\n', '\r', '\0', 'é', 'Ж', '中', '😀',
    '\u{0301}', '\u{2028}',
];

const STRING_CHARACTERS: &[char] = &[
    'a', 'Z', '0', ' ', '"', '\\', '/', '\t', '\n', '\r', '\0', 'é', 'Ж', '中', '😀', '\u{0301}',
    '\u{2028}',
];

fn random_text(rng: &mut DeterministicRng, max_characters: usize) -> String {
    let length = rng.below(max_characters + 1);
    (0..length)
        .map(|_| TEXT_CHARACTERS[rng.below(TEXT_CHARACTERS.len())])
        .collect()
}

fn random_string(rng: &mut DeterministicRng) -> String {
    let length = rng.below(12);
    (0..length)
        .map(|_| STRING_CHARACTERS[rng.below(STRING_CHARACTERS.len())])
        .collect()
}

fn valid_json(rng: &mut DeterministicRng, depth: usize) -> String {
    let variants = if depth == 0 { 5 } else { 7 };
    match rng.below(variants) {
        0 => "null".to_owned(),
        1 => ["true", "false"][rng.below(2)].to_owned(),
        2 => [
            "0",
            "-0",
            "17",
            "-42",
            "3.1415",
            "6.02e23",
            "1E-9",
            "9.999e+125",
        ][rng.below(8)]
        .to_owned(),
        3 | 4 => serde_json::to_string(&random_string(rng)).expect("strings are serializable"),
        5 => {
            let values = (0..rng.below(5))
                .map(|_| valid_json(rng, depth - 1))
                .collect::<Vec<_>>();
            format!("[{}]", values.join(","))
        }
        6 => {
            let members = (0..rng.below(5))
                .map(|_| {
                    let key = serde_json::to_string(&random_string(rng))
                        .expect("object keys are serializable");
                    format!("{key}:{}", valid_json(rng, depth - 1))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", members.join(","))
        }
        _ => unreachable!(),
    }
}

fn mutate_common_json(rng: &mut DeterministicRng) -> String {
    let mut characters = valid_json(rng, 4).chars().collect::<Vec<_>>();
    match rng.below(6) {
        0 => {}
        1 => characters.push(','),
        2 => characters.extend(" null".chars()),
        3 if !characters.is_empty() => {
            let index = rng.below(characters.len());
            characters.remove(index);
        }
        4 if !characters.is_empty() => {
            let index = rng.below(characters.len());
            characters.insert(index, '@');
        }
        5 if !characters.is_empty() => {
            let index = rng.below(characters.len());
            characters[index] = [']', '}', ':', '"'][rng.below(4)];
        }
        _ => characters.push('@'),
    }
    characters.into_iter().collect()
}

fn assert_lossless_and_safe(source: &str) {
    let lexed = lex(source);
    let mut cursor = 0;
    for token in lexed.tokens() {
        assert_eq!(token.span.start, cursor, "gap or overlap in {source:?}");
        assert!(
            token.span.start < token.span.end,
            "empty token in {source:?}"
        );
        assert!(
            token.span.is_valid_for(source),
            "invalid span in {source:?}"
        );
        assert_eq!(token.text(source), source.get(token.span.range()));
        cursor = token.span.end;
    }
    assert_eq!(cursor, source.len(), "tokens do not cover {source:?}");

    for diagnostic in lexed.diagnostics() {
        assert!(
            diagnostic.span.is_valid_for(source),
            "invalid lex diagnostic span in {source:?}"
        );
    }

    let parsed = parse(source);
    for diagnostic in parsed.diagnostics() {
        assert!(
            diagnostic.span.is_valid_for(source),
            "invalid parse diagnostic span in {source:?}"
        );
    }

    let semantic = tokenize(source);
    assert_eq!(semantic.tokens().len(), lexed.tokens().len());
    let mut semantic_cursor = 0;
    for token in semantic.tokens() {
        assert_eq!(
            token.span.start, semantic_cursor,
            "semantic gap in {source:?}"
        );
        assert!(token.span.start < token.span.end, "empty semantic token");
        assert!(token.span.is_valid_for(source));
        assert!(token.text(source).is_some());
        semantic_cursor = token.span.end;
    }
    assert_eq!(semantic_cursor, source.len());
}

#[test]
fn arbitrary_utf8_is_lossless_span_safe_and_panic_free() {
    let mut rng = DeterministicRng::new(0x6a09_e667_f3bc_c909);
    for case_index in 0..4_096 {
        let source = random_text(&mut rng, 96);
        let outcome = catch_unwind(AssertUnwindSafe(|| assert_lossless_and_safe(&source)));
        assert!(
            outcome.is_ok(),
            "property failure for deterministic case {case_index}: {source:?}"
        );
    }
}

#[test]
fn generated_valid_json_is_accepted_and_lossless() {
    let mut rng = DeterministicRng::new(0xbb67_ae85_84ca_a73b);
    for case_index in 0..2_048 {
        let source = valid_json(&mut rng, 4);
        assert!(
            parse(&source).is_valid(),
            "generated case {case_index} was rejected: {source:?}"
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&source).is_ok(),
            "serde_json rejected generated case {case_index}: {source:?}"
        );
        assert_lossless_and_safe(&source);
    }
}

#[test]
fn strict_validity_matches_serde_json_for_deterministic_inputs() {
    let mut rng = DeterministicRng::new(0x3c6e_f372_fe94_f82b);
    let fixed_cases = [
        "",
        "null",
        " true ",
        "{\"x\":[1,false,null]}",
        "{\"x\":1,}",
        "[1 2]",
        "\u{feff}null",
        "// comment\nnull",
        "\"\\uD834\\uDD1E\"",
        "\"\\uD800\"",
    ];

    for (case_index, source) in fixed_cases
        .into_iter()
        .map(str::to_owned)
        .chain((0..8_192).map(|_| mutate_common_json(&mut rng)))
        .enumerate()
    {
        let ours = parse(&source).is_valid();
        let reference = serde_json::from_str::<serde_json::Value>(&source).is_ok();
        assert_eq!(
            ours, reference,
            "strict-validity mismatch for deterministic case {case_index}: {source:?}"
        );
    }
}
