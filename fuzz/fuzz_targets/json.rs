#![no_main]

use libfuzzer_sys::fuzz_target;
use themoretheless_tokenizer::{
    Span,
    json::{ParseOptions, Value, lex_with, parse_with, semantic_tokens},
};

const INPUT_LIMITS: [usize; 5] = [0, 1, 2, 64, 64 * 1024];
const TOKEN_LIMITS: [usize; 5] = [0, 1, 2, 64, 4_096];
const DIAGNOSTIC_LIMITS: [usize; 4] = [0, 1, 2, 64];
const DEPTH_LIMITS: [usize; 6] = [0, 1, 32, 127, 128, usize::MAX];

fuzz_target!(|data: &[u8]| {
    let (controls, text) = if data.len() >= 4 {
        data.split_at(4)
    } else {
        (&[0, 0, 0, 0][..], data)
    };
    let Ok(source) = std::str::from_utf8(text) else {
        return;
    };

    let max_input_bytes = match controls[0] % 6 {
        0 => source.len().saturating_sub(1),
        index => INPUT_LIMITS[usize::from(index - 1)],
    };
    let max_tokens = TOKEN_LIMITS[usize::from(controls[1]) % TOKEN_LIMITS.len()];
    let max_diagnostics = DIAGNOSTIC_LIMITS[usize::from(controls[2]) % DIAGNOSTIC_LIMITS.len()];
    let max_depth = DEPTH_LIMITS[usize::from(controls[3]) % DEPTH_LIMITS.len()];

    check_mode(
        source,
        ParseOptions::strict(),
        max_input_bytes,
        max_tokens,
        max_diagnostics,
        max_depth,
    );
    check_mode(
        source,
        ParseOptions::jsonc(),
        max_input_bytes,
        max_tokens,
        max_diagnostics,
        max_depth,
    );

    check_depth_boundary(controls[0]);
});

fn check_mode(
    source: &str,
    options: ParseOptions,
    max_input_bytes: usize,
    max_tokens: usize,
    max_diagnostics: usize,
    max_depth: usize,
) {
    let options = options
        .max_input_bytes(max_input_bytes)
        .max_tokens(max_tokens)
        .max_diagnostics(max_diagnostics)
        .max_depth(max_depth);

    let lexed = lex_with(source, options.lexer_options());
    assert_eq!(lexed.source(), source);
    assert_lossless_spans(source, lexed.tokens().iter().map(|token| token.span));
    assert!(lexed.tokens().len() <= max_tokens.saturating_add(1));
    assert!(lexed.diagnostics().len() <= max_diagnostics.saturating_add(1));
    assert!(
        lexed
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.span.is_valid_for(source))
    );

    let parsed = parse_with(source, options);
    assert_eq!(parsed.source(), source);
    assert_eq!(parsed.lexed(), &lexed);
    assert!(parsed.diagnostics().len() <= max_diagnostics.saturating_add(1));
    assert!(parsed.property_spans().len() <= lexed.tokens().len());
    assert!(parsed.invalid_value_spans().len() <= lexed.tokens().len());
    assert!(
        parsed
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.span.is_valid_for(source))
    );
    assert!(
        parsed
            .property_spans()
            .iter()
            .chain(parsed.invalid_value_spans())
            .all(|span| span.is_valid_for(source))
    );
    if let Some(value) = parsed.value() {
        assert_value_spans(source, value);
    }

    let semantic = semantic_tokens(&parsed);
    assert_eq!(semantic.diagnostics(), parsed.diagnostics());
    assert_eq!(semantic.tokens().len(), parsed.lexed().tokens().len());
    assert_lossless_spans(source, semantic.tokens().iter().map(|token| token.span));
}

fn check_depth_boundary(selector: u8) {
    let depth = [127, 128, 129][usize::from(selector) % 3];
    let mut source = "[".repeat(depth);
    source.push_str("null");
    source.push_str(&"]".repeat(depth));
    let parsed = parse_with(&source, ParseOptions::strict().max_depth(usize::MAX));
    if depth <= 128 {
        assert!(parsed.is_valid());
    } else {
        assert!(parsed.has_errors());
    }
    drop(parsed);
}

fn assert_lossless_spans(source: &str, spans: impl IntoIterator<Item = Span>) {
    let mut end = 0;
    for span in spans {
        assert!(span.is_valid_for(source));
        assert!(!span.is_empty());
        assert_eq!(span.start, end);
        assert!(span.slice(source).is_some());
        end = span.end;
    }
    assert_eq!(end, source.len());
}

fn assert_value_spans(source: &str, value: &Value<'_>) {
    let span = value.span();
    assert!(span.is_valid_for(source));
    assert!(!span.is_empty());

    match value {
        Value::Object(object) => {
            for member in object.members() {
                assert!(member.span().is_valid_for(source));
                assert!(member.key().span().is_valid_for(source));
                assert_eq!(
                    member.key().raw(),
                    member.key().span().slice(source).unwrap()
                );
                assert_value_spans(source, member.value());
            }
        }
        Value::Array(array) => {
            for element in array.elements() {
                assert_value_spans(source, element);
            }
        }
        Value::String(string) => {
            assert_eq!(string.raw(), span.slice(source).unwrap());
        }
        Value::Number(number) => {
            assert_eq!(number.as_str(), span.slice(source).unwrap());
        }
        Value::Boolean(_) | Value::Null(_) => {}
        _ => {}
    }
}
