use wasm_bindgen::prelude::*;
#[wasm_bindgen]
pub fn parse_json(source: &str, fragment: bool) -> Result<String, JsValue> {
    themoretheless_tokenizer_forma::parse_with_options(source, fragment)
        .map(|v| v.to_string())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
#[wasm_bindgen]
pub fn parse_expression_json(source: &str) -> Result<String, JsValue> {
    themoretheless_tokenizer_forma::parse_expression(source)
        .map(|v| v.to_string())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
use serde_json::{Value as V, json};
use themoretheless_tokenizer_forma::Diagnostic;
/// Errors reach the host as a JS Error whose `message` is the previous plain
/// string; `code` and `span` are additive fields for Studio diagnostics.
fn diagnostic_to_js(e: &Diagnostic) -> JsValue {
    let err = js_sys::Error::new(&e.message);
    let _ = js_sys::Reflect::set(&err, &"code".into(), &JsValue::from_str(e.code));
    if let Some(span) = e.span {
        let _ = js_sys::Reflect::set(&err, &"span".into(), &JsValue::from_f64(span as f64));
    }
    err.into()
}
fn callback(f: &js_sys::Function, path: &str, optional: bool) -> Result<V, Diagnostic> {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"optional".into(), &optional.into())
        .map_err(|_| Diagnostic::new("resolver-error", "Resolver options"))?;
    let value = f.call2(&JsValue::NULL, &path.into(), &opts).map_err(|e| {
        Diagnostic::new(
            "resolver-error",
            e.as_string().unwrap_or_else(|| {
                js_sys::Reflect::get(&e, &"message".into())
                    .ok()
                    .and_then(|m| m.as_string())
                    .unwrap_or_else(|| "Reference resolution failed".into())
            }),
        )
    })?;
    if value.is_undefined() || value.is_null() {
        return Ok(V::Null);
    }
    if let Some(b) = value.as_bool() {
        return Ok(V::Bool(b));
    }
    if let Some(n) = value.as_f64() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return Ok(V::Number(num));
        }
        return Err("Resolver returned non-serializable data".into());
    }
    if let Some(s) = value.as_string() {
        return Ok(V::String(s));
    }
    let s = js_sys::JSON::stringify(&value)
        .map_err(|_| Diagnostic::new("resolver-error", "Resolver returned non-serializable data"))?
        .as_string()
        .ok_or_else(|| Diagnostic::new("resolver-error", "Resolver returned non-serializable data"))?;
    serde_json::from_str(&s).map_err(|e| Diagnostic::new("invalid-input", e.to_string()))
}
#[wasm_bindgen]
pub fn operation_json(
    operation: &str,
    input: &str,
    resolver: Option<js_sys::Function>,
) -> Result<String, JsValue> {
    let result = (|| -> Result<V, Diagnostic> {
        let v: V = serde_json::from_str(input).map_err(|e| {
            if e.to_string().contains("recursion limit") {
                Diagnostic::new("depth-limit", "Превышен предел глубины разметки")
            } else {
                Diagnostic::new("invalid-input", e.to_string())
            }
        })?;
        let arg = |k: &str| &v[k];
        let empty = json!({});
        let state = arg("state");
        let props = arg("props");
        let env = v.get("environment").unwrap_or(&empty);
        use themoretheless_tokenizer_forma::{evaluate as e, frontend as f, semantics as s};
        match operation{
 "tokenize"=>Ok(json!(f::tokenize(v["source"].as_str().unwrap_or("")).map_err(Diagnostic::from)?.iter().map(|t|json!({"v":t.text,"pos":t.start+v["offset"].as_u64().unwrap_or(0)as usize})).collect::<Vec<_>>())),
 "serialize"=>Ok(json!(f::serialize_value(arg("value")))),"references"=>Ok(json!(e::references(arg("value")))),"equal"=>Ok(json!(e::equal(arg("a"),arg("b")))),
 "evaluateExpression"=>e::evaluate(arg("value"),&mut |path,opt|callback(resolver.as_ref().ok_or_else(||Diagnostic::new("resolver-error","Missing reference resolver"))?,path,opt)),
 "matchesPattern"=>Ok(json!(e::matches_pattern(arg("pattern"),arg("value"),&mut |p|if let Some(r)=resolver.as_ref(){let input=js_sys::JSON::parse(&p.to_string()).map_err(|_|Diagnostic::new("conversion-error","Pattern conversion"))?;let result=r.call1(&JsValue::NULL,&input).map_err(|e|Diagnostic::new("resolver-error",e.as_string().unwrap_or_else(||"Pattern resolver failed".into())))?;let text=js_sys::JSON::stringify(&result).map_err(|_|Diagnostic::new("conversion-error","Pattern conversion"))?.as_string().unwrap_or_else(||"null".into());serde_json::from_str(&text).map_err(|e|Diagnostic::new("invalid-input",e.to_string()))}else{Ok(p.clone())})?)),
 "evaluate"=>s::evaluate_value(arg("value"),props,state,&v["stack"].as_array().map(|a|a.iter().filter_map(|v|v.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default(),v["expandElements"]!=false,env),
 "evaluateProperties"=>s::evaluate_properties(arg("node"),props,state,env),"selectedProperties"=>s::selected_properties(arg("groups"),props,state,env),"selectedPropertySources"=>s::selected_sources(arg("groups"),props,state,env),
 "expandStructure"=>{let track_origins=arg("options")["trackOrigins"]==true;s::expand_structure(arg("nodes"),props,state,env,arg("options"),&mut |node,environment|{
 let Some(r)=resolver.as_ref()else{return Ok(V::Null)};let n=js_sys::JSON::parse(&node.to_string()).map_err(|_|Diagnostic::new("conversion-error","Node conversion"))?;let env_js=if track_origins{js_sys::JSON::parse(&environment.to_string()).map_err(|_|Diagnostic::new("conversion-error","Environment conversion"))?}else{JsValue::NULL};let value=r.call2(&JsValue::NULL,&n,&env_js).map_err(|e|Diagnostic::new("resolver-error",js_sys::Reflect::get(&e,&"message".into()).ok().and_then(|v|v.as_string()).unwrap_or_else(||"Markup host hook failed".into())))?;let text=js_sys::JSON::stringify(&value).map_err(|_|Diagnostic::new("conversion-error","Annotation conversion"))?.as_string().unwrap_or_else(||"{}".into());serde_json::from_str(&text).map_err(|e|Diagnostic::new("invalid-input",e.to_string()))
 })},
 "validateDesign"=>{s::validate_design(arg("component"),arg("design"))?;Ok(V::Null)},
 "designStatePatch"=>s::design_state_patch(arg("design"),v["stateName"].as_str()),
 "matchesType"=>Ok(json!(s::matches_type(arg("value"),v["type"].as_str().unwrap_or(""),arg("enums")))),"validateTypeName"=>{s::validate_type_name(v["type"].as_str().unwrap_or(""),arg("enums"))?;Ok(V::Null)},"validateContract"=>{s::validate_contract(arg("definitions"),arg("values"),arg("enums"),v["label"].as_str().unwrap_or("component"))?;Ok(V::Null)},
 "format"=>Ok(json!(f::format_source(v["source"].as_str().unwrap_or("")).map_err(Diagnostic::from)?)),
 "lint"=>f::lint_document(v["source"].as_str().unwrap_or("")).map_err(Diagnostic::from),
 _=>Err(Diagnostic::new("unknown-operation",format!("Unknown Forma operation {operation}")))
 }
    })();
    result.map(|v| v.to_string()).map_err(|e| diagnostic_to_js(&e))
}
