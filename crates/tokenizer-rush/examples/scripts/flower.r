// Run with radius=30 petals=6 time=0. Angles are radians after deg().
fn point(angle) {
    const distance = radius + radius / 3 * cos(deg(angle * petals))
    return vec2(cos(deg(angle)), sin(deg(angle))) * distance
}
range(0, 360, 5)
    | map(point)
    | polygon
    | rotate(time)
