use themoretheless_tokenizer::{
    ColumnEncoding, JsonTokenizer, LineColumn, LineIndex, Span, TokenKind, Tokenizer, UrlKind,
    json::{
        AstPathSegment, AstVisitor, LexerOptions, NodeRef, NumberError, Parse, ParseOptions,
        SemanticKind, SyntaxElement, SyntaxKind, SyntaxNodeKind, TextEdit, Value, VisitContext,
        VisitControl, VisitOutcome, apply_edits, lex_with, node_at_offset, parse, parse_with,
        path_at_offset, syntax_tree, tokenize, visit_parse,
    },
    tokenize_url, validate_url,
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
fn url_tokenizer_is_on_the_public_surface() {
    let source = "https://api.example.com/v1?q=1&x=2#top";
    let tokens = tokenize_url(source);
    assert!(tokens.is_lossless(source));
    assert!(tokens.tokens.iter().any(|t| {
        t.kind == UrlKind::Host && t.text(source) == Some("api.example.com")
    }));
    assert!(tokens.tokens.iter().any(|t| t.kind == UrlKind::Key && t.text(source) == Some("q")));
    let diagnostics = validate_url("https://h:70000/ path");
    assert!(diagnostics.iter().any(|d| d.code == "url-whitespace"));
    assert!(diagnostics.iter().any(|d| d.code == "url-port-range"));
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

#[test]
fn navigation_api_exposes_sound_borrowed_nodes_and_paths() {
    fn navigate<'ast, 'source>(
        parsed: &'ast Parse<'source>,
        offset: usize,
    ) -> (NodeRef<'ast, 'source>, Vec<AstPathSegment<'ast, 'source>>) {
        (
            node_at_offset(parsed, offset).unwrap().unwrap(),
            path_at_offset(parsed, offset)
                .unwrap()
                .unwrap()
                .into_segments(),
        )
    }

    let source = r#"{"x":[{"name":"first"}],"x":2}"#;
    let parsed = parse(source);
    let offset = source.find("first").unwrap();
    let (node, path) = navigate(&parsed, offset);

    assert_eq!(node.as_value().and_then(Value::as_str), Some("first"));
    assert!(matches!(
        path.as_slice(),
        [
            AstPathSegment::ObjectMember {
                member_index: 0,
                ..
            },
            AstPathSegment::ArrayElement { index: 0 },
            AstPathSegment::ObjectMember {
                member_index: 0,
                ..
            },
        ]
    ));
}

#[test]
fn visitor_api_is_usable_from_downstream_code() {
    #[derive(Default)]
    struct Keys(Vec<String>);

    impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for Keys {
        fn enter_value(
            &mut self,
            _value: &'ast Value<'source>,
            context: VisitContext<'ast, 'source>,
        ) -> VisitControl {
            if let VisitContext::ObjectMember { member, .. } = context {
                self.0
                    .push(member.key().decoded().unwrap_or("invalid").to_owned());
            }
            VisitControl::Continue
        }
    }

    let parsed = parse(r#"{"a":1,"a":{"b":2}}"#);
    let mut keys = Keys::default();
    assert_eq!(visit_parse(&parsed, &mut keys), VisitOutcome::Completed);
    assert_eq!(keys.0, ["a", "a", "b"]);
}

#[test]
fn syntax_tree_and_edit_api_are_usable_downstream() {
    let source = r#"{"name":"Мир"}"#;
    let tree = syntax_tree(source);
    assert_eq!(tree.source(), source);
    assert_eq!(tree.root().kind(), SyntaxNodeKind::Root);

    let root_value = tree
        .root()
        .elements()
        .iter()
        .find_map(|element| match element {
            SyntaxElement::Node(id) => tree.node(*id),
            SyntaxElement::Token(_) => None,
            _ => None,
        })
        .unwrap();
    assert_eq!(root_value.kind(), SyntaxNodeKind::Object);
    assert_eq!(root_value.parent(), Some(tree.root_id()));

    let start = source.find("Мир").unwrap();
    let edited = tree
        .apply_edits(&[TextEdit::new(Span::new(start, start + "Мир".len()), "世界")])
        .unwrap();
    assert_eq!(edited, r#"{"name":"世界"}"#);
    assert_eq!(apply_edits("abc", &[]).unwrap(), "abc");
}
