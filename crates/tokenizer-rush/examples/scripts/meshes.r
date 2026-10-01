// Concatenate mesh components, preserving faces and offsetting the second indices.
// This does not perform a boolean union or weld coincident vertices.
fn join(a: mesh, b: mesh) -> mesh {
    const av = a.vertices
    const bv = b.vertices
    const offset = len(av)
    const vertices = [av, bv] | flat_map(xs => xs)
    const shifted = b.triangles | map(face => face | map(i => i + offset))
    const triangles = [a.triangles, shifted] | flat_map(xs => xs)
    return mesh(vertices, triangles)
}
{join: join}
