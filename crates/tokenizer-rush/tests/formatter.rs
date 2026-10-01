use themoretheless_tokenizer_rush::{evaluate, format_source, parse};

fn stable(source: &str) -> String {
    let formatted = format_source(source).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    assert!(parse(&formatted).is_valid(), "{formatted}");
    assert_eq!(
        format_source(&formatted).unwrap(),
        formatted,
        "not stable: {source}"
    );
    formatted
}

#[test]
fn canonical_blocks_calls_and_bindings() {
    assert_eq!(
        stable("fn add a:number b:number -> number:\n  return a+b\nconst value=add 1 2\nvalue"),
        "fn add(a: number, b: number) -> number {\n    return a + b;\n}\nlet value = add(1, 2);\nvalue;\n"
    );
    assert_eq!(stable(""), "");
    assert!(format_source("let x =").is_err());
}

#[test]
fn precedence_and_associativity_preserve_execution() {
    for source in [
        "(2 ** 3) ** 2",
        "2 ** (3 ** 2)",
        "-2 ** 2",
        "(-2) ** 2",
        "10 - (3 - 1)",
        "10 / (2 * 5)",
        "(1 + 2) * 3",
        "not (false or true)",
        "let f = x => x+1\nf(3)",
        "((x,y) => x+y)(1,2)",
        "let f = ((x,y)) => x+y\nf((1,2))",
        "(if true { 1 } else { 2 }) + 3",
        "match Some(3) { Some(x) if (x>0) => x+1, _ => 0 }",
        "mut x = 1\nx += 2\nx",
        "let (a,(b,_)) = (1,(2,3))\na+b",
        "let r={point:(2,3)}\nlet f=({point:(x,y)})=>x+y\nf(r)",
        "[1,2,3] | map(x=>x*2) | fold(0,(a,b)=>a+b)",
        "mut s=0\nfor x in range_iter(0,5) { if x==1 {continue}; s+=x }\ns",
    ] {
        let formatted = stable(source);
        assert_eq!(
            evaluate(source, 2000).unwrap(),
            evaluate(&formatted, 2000).unwrap(),
            "{source}\n{formatted}"
        );
    }
}

#[test]
fn comments_and_literal_spellings_survive() {
    for source in [
        "// leading\nconst x=1 // trailing\nx",
        "let x=(1 + // addition\n2)\nx",
        "fn f(x /* parameter */) { // body\nreturn (x // keep\n+ 1) }\nf(2)",
        "let s='// is a string'\n/* end */\ns",
        "match Some(1) { // arms\nSome(x) => x, // fallback\n_ => 0 }",
        "fn f():\n    // body comment\n    return 2\n// outside\nf()",
        "let values = [1, /* nested\ncomment */ 2]\nvalues",
    ] {
        let formatted = stable(source);
        let comments = |s: &str| {
            parse(s)
                .lexed
                .tokens
                .into_iter()
                .filter(|t| {
                    matches!(
                        t.kind,
                        themoretheless_tokenizer_core::SyntaxKind::LineComment
                            | themoretheless_tokenizer_core::SyntaxKind::BlockComment
                    )
                })
                .map(|t| s[t.span.start..t.span.end].to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(comments(source), comments(&formatted));
        assert_eq!(
            evaluate(source, 2000).unwrap(),
            evaluate(&formatted, 2000).unwrap()
        );
    }
}

#[test]
fn repository_script_corpus_formats_stably() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/scripts");
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "r") {
            stable(&std::fs::read_to_string(path).unwrap());
        }
    }
}
