use themoretheless_tokenizer::{
    ColumnEncoding, JsonTokenizer, LineColumn, LineIndex, Span, TokenKind, Tokenizer,
    json::{
        LexerOptions, NumberError, ParseOptions, SemanticKind, SyntaxKind, Value, lex_with, parse,
        parse_with, tokenize,
    },
};

#[test]
fn legacy_api_remains_source_compatible() {
    let source = r#"{"name":"Denis"}"#;
    let result = JsonTokenizer.tokenize(source);
    assert!(result.diagnostics.is_empty());
    assert!(result.tokens.iter().any(|token| {
        token.kind == TokenKind::Property && token.text(source) == Some("\"name\"")
    }));
}

#[test]
fn curated_json_facade_is_sufficient_for_consumers() {
    let source = "{/* note */\"n\":9007199254740993,}";
    let parsed = parse_with(source, ParseOptions::jsonc());
    let object = parsed.value().and_then(Value::as_object).unwrap();
    let number = object.get("n").and_then(Value::as_number).unwrap();
    assert!(number.is_valid());
    assert_eq!(number.as_i64(), Ok(9_007_199_254_740_993));

    let lexed = lex_with(source, LexerOptions::jsonc());
    assert!(
        lexed
            .tokens()
            .iter()
            .any(|token| token.kind == SyntaxKind::BlockComment)
    );
}

#[test]
fn object_lookup_result_does_not_borrow_the_lookup_key() {
    fn first_x<'source>(value: &'source Value<'source>) -> &'source Value<'source> {
        let key = String::from("x");
        value.as_object().unwrap().get_all(&key).next().unwrap()
    }

    let parsed = parse(r#"{"x":1,"x":2}"#);
    assert_eq!(
        first_x(parsed.value().unwrap())
            .as_number()
            .unwrap()
            .as_i64(),
        Ok(1)
    );
}

#[test]
fn malformed_numbers_are_not_convertible() {
    let parsed = parse("01");
    let number = parsed.value().and_then(Value::as_number).unwrap();
    assert_eq!(number.as_i64(), Err(NumberError::InvalidJsonNumber));
}

#[test]
fn source_and_semantic_apis_use_safe_spans() {
    let source = "😀\r\n{\"x\":true}";
    let index = LineIndex::new(source);
    assert_eq!(
        index.line_column("😀\r\n".len(), ColumnEncoding::Utf16CodeUnits),
        Ok(LineColumn::new(1, 0))
    );

    let semantic_source = &source["😀\r\n".len()..];
    let semantic = tokenize(semantic_source);
    assert!(semantic.tokens().iter().any(|token| {
        token.kind == SemanticKind::Property
            && token.span == Span::new(1, 4)
            && token.text(semantic_source) == Some("\"x\"")
    }));
}
