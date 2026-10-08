use themoretheless_tokenizer_rush::{ReplCommand, ReplOutcome, ReplSession, is_input_complete};

#[test]
fn test_repl_input_completeness() {
    assert!(is_input_complete("1 + 2"));
    assert!(is_input_complete("let x = [1, 2, 3]"));
    assert!(!is_input_complete("fn f(x) {"));
    assert!(!is_input_complete("let arr = ["));
    assert!(!is_input_complete("let s = \"unclosed string"));
    assert!(is_input_complete("fn f(x) {\n    return x\n}"));
}

#[test]
fn test_repl_expressions_and_variables() {
    let mut session = ReplSession::new().expect("session created");

    assert_eq!(session.eval_line("1 + 2"), ReplOutcome::Value("3".into()));

    assert_eq!(session.eval_line("let x = 10"), ReplOutcome::Void);
    assert_eq!(session.eval_line("x * 3"), ReplOutcome::Value("30".into()));

    assert_eq!(session.eval_line("mut count = 1"), ReplOutcome::Void);
    assert_eq!(
        session.eval_line("count += 5"),
        ReplOutcome::Value("6".into())
    );
    assert_eq!(session.eval_line("count"), ReplOutcome::Value("6".into()));
}

#[test]
fn test_repl_multiline_and_functions() {
    let mut session = ReplSession::new().expect("session created");

    assert_eq!(session.eval_line("fn add(a, b) {"), ReplOutcome::Incomplete);
    assert_eq!(
        session.eval_line("    return a + b"),
        ReplOutcome::Incomplete
    );
    assert_eq!(session.eval_line("}"), ReplOutcome::Void);

    assert_eq!(
        session.eval_line("add(10, 20)"),
        ReplOutcome::Value("30".into())
    );
}

#[test]
fn test_repl_closure_capturing_earlier_variable() {
    let mut session = ReplSession::new().expect("session created");

    session.eval_line("let multiplier = 4");
    session.eval_line("fn multiply(n) {\n    return n * multiplier\n}");
    assert_eq!(
        session.eval_line("multiply(5)"),
        ReplOutcome::Value("20".into())
    );
}

#[test]
fn test_repl_json_integration() {
    let mut session = ReplSession::new().expect("session created");

    session.eval_line("let parsed = json_parse('{\"key\": 42}')");
    assert_eq!(
        session.eval_line("parsed"),
        ReplOutcome::Value("Ok({ key: 42 })".into())
    );
    session.eval_line("let obj = match parsed { Ok(v) => v, Err(e) => { key: 0 } }");
    assert_eq!(
        session.eval_line("obj.key"),
        ReplOutcome::Value("42".into())
    );
}

#[test]
fn test_repl_commands_and_reset() {
    let mut session = ReplSession::new().expect("session created");

    session.eval_line("let a = 1");
    session.eval_line("let b = 2");

    match session.eval_line(":vars") {
        ReplOutcome::Command(ReplCommand::Vars(vars)) => {
            assert!(vars.iter().any(|(k, v)| k == "a" && v == "1"));
            assert!(vars.iter().any(|(k, v)| k == "b" && v == "2"));
        }
        other => panic!("expected Vars command, got {other:?}"),
    }

    assert_eq!(
        session.eval_line(":help"),
        ReplOutcome::Command(ReplCommand::Help)
    );
    assert_eq!(
        session.eval_line("exit"),
        ReplOutcome::Command(ReplCommand::Exit)
    );

    assert_eq!(
        session.eval_line(":reset"),
        ReplOutcome::Command(ReplCommand::Reset)
    );
    // After reset, previous variables are cleared
    match session.eval_line("a") {
        ReplOutcome::Error(msg) => assert!(msg.contains("Unknown name")),
        other => panic!("expected error for cleared variable, got {other:?}"),
    }
}

#[test]
fn test_repl_error_recovery() {
    let mut session = ReplSession::new().expect("session created");

    session.eval_line("let valid = 100");

    // Runtime error
    let err = session.eval_line("unknown_var");
    assert!(matches!(err, ReplOutcome::Error(_)));

    // Syntax error
    let syntax_err = session.eval_line("+++");
    assert!(matches!(syntax_err, ReplOutcome::Error(_)));

    // Session is still alive and previous bindings are intact
    assert_eq!(session.eval_line("valid"), ReplOutcome::Value("100".into()));
}
