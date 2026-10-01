// A ruled ribbon between two cubic curves. Output is an OBJ triangle mesh.
import curves
fn edge(t) {
    return curves.cubic(vec3(-3,0,0), vec3(-1,3,1), vec3(1,-3,1), vec3(3,0,0), t)
}
grid_mesh(range(0, 41), [0, 1], (i, side) => edge(i / 40) + vec3(0, 0, side * 0.5))
