use themoretheless_tokenizer_rush::{Value, evaluate};
fn main() {
    let source = "range(0, 360, 5) | map(a => vec2(cos(deg(a)), sin(deg(a))) * (30 + 10 * cos(deg(a * 6)))) | polygon";
    let Value::Polygon(polygon) = evaluate(source, 10000).expect("valid script") else {
        panic!("expected polygon")
    };
    print!("{}", polygon.to_svg().expect("valid SVG"));
}
