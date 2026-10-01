// Curve evaluation works with numbers and vectors through lerp.
fn polar(radius: number, angle: number) -> vec2 {
    return vec2(cos(angle), sin(angle)) * radius
}

fn quadratic(a, b, c, t: number) {
    return lerp(lerp(a, b, t), lerp(b, c, t), t)
}

fn cubic(a, b, c, d, t: number) {
    return lerp(quadratic(a, b, c, t), quadratic(b, c, d, t), t)
}

// Analytic tangents preserve magnitude; normalize explicitly for directions.
fn quadratic_tangent(a, b, c, t: number) {
    return lerp(b - a, c - b, t) * 2
}

fn cubic_tangent(a, b, c, d, t: number) {
    return quadratic(b - a, c - b, d - c, t) * 3
}

{polar: polar, quadratic: quadratic, cubic: cubic,
 quadratic_tangent: quadratic_tangent, cubic_tangent: cubic_tangent}
