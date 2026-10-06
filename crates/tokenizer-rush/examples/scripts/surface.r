// Run with amplitude=2. The runner exports this mesh as OBJ.
fn point(x, y) -> vec3 {
    return vec3(x, y, amplitude * sin(x) * cos(y))
}
grid_mesh(range(-5, 5.1, 0.25), range(-5, 5.1, 0.25), point)
