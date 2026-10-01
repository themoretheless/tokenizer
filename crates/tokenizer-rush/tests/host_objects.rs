use std::{cell::RefCell, rc::Rc};
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, HostObject, Program, Value, ValueType as T,
    analyze_editor_with_host,
};

thread_local! { static OWNER: RefCell<Option<Rc<f64>>> = const { RefCell::new(None) }; }
fn object<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    OWNER.with(|owner| {
        Ok(Value::HostObject(HostObject::new(
            "Point",
            owner.borrow().as_ref().unwrap(),
        )))
    })
}
fn read<'s>(values: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::HostObject(object) = &values[0] else {
        unreachable!()
    };
    Ok(Value::Number(
        *object
            .upgrade::<f64>()
            .expect("contract rejects expired objects"),
    ))
}
fn release<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    OWNER.with(|owner| {
        owner.borrow_mut().take();
    });
    Ok(Value::Null)
}
fn functions() -> Vec<Rc<HostFunction>> {
    vec![
        Rc::new(HostFunction {
            name: "point",
            parameters: vec![],
            result: T::HostObject("Point"),
            callback: object,
        }),
        Rc::new(HostFunction {
            name: "read",
            parameters: vec![T::HostObject("Point")],
            result: T::Number,
            callback: read,
        }),
        Rc::new(HostFunction {
            name: "release",
            parameters: vec![],
            result: T::Null,
            callback: release,
        }),
    ]
}
#[test]
fn aliases_preserve_identity_without_owning_the_object() {
    let owner = Rc::new(42.);
    let reference = HostObject::new("Point", &owner);
    let alias = HostObject::new("Point", &owner);
    assert_eq!(reference, alias);
    assert_ne!(reference, HostObject::new("Point", &Rc::new(42.)));
    assert!(reference.upgrade::<String>().is_none());
    assert!(T::HostObject("Point").accepts(&Value::HostObject(reference.clone())));
    assert!(!T::HostObject("Mesh").accepts(&Value::HostObject(reference.clone())));
    let pinned = reference.upgrade::<f64>().unwrap();
    drop(owner);
    assert!(reference.is_alive());
    drop(pinned);
    assert!(!reference.is_alive());
    assert!(reference.upgrade::<f64>().is_none());
    assert_eq!(reference, alias);
    assert!(!T::HostObject("Point").accepts(&Value::HostObject(reference)));
}
#[test]
fn runtime_checks_liveness_and_types_before_invoking_host() {
    let functions = functions();
    OWNER.with(|owner| *owner.borrow_mut() = Some(Rc::new(42.)));
    let source = "let p = point(); assert(p == point()); let alias = p; read(alias)";
    assert!(
        analyze_editor_with_host(source, &[], &functions)
            .parsed
            .is_valid()
    );
    assert_eq!(
        Program::compile(source)
            .unwrap()
            .run_with_host(1000, &Default::default(), &[], &functions)
            .unwrap(),
        Value::Number(42.)
    );
    let source = "let p = point(); let saved = () => read(p); release(); saved()";
    let error = Program::compile(source)
        .unwrap()
        .run_with_host(1000, &Default::default(), &[], &functions)
        .unwrap_err();
    assert_eq!(&source[error.span.start..error.span.end], "read(p)");
    assert!(error.message.contains("argument") || error.message.contains("Argument"));
    assert!(
        !analyze_editor_with_host("read(1)", &[], &functions)
            .parsed
            .is_valid()
    );
}
