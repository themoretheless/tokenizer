import meshes
const triangle = mesh([vec3(0,0,0), vec3(1,0,0), vec3(0,1,0)], [[0,1,2]])
const lifted = transform(triangle, translation(vec3(0,0,2)))
const pair = meshes.join(triangle, lifted)
assert(len(pair.vertices) == 6, 'joined vertex count')
assert(pair.triangles == [[0,1,2], [3,4,5]], 'joined indices')
assert(all(lifted.vertices, v => v.z == 2), 'transformed vertices')
assert(all(triangle.vertices, v => v.z == 0), 'original preserved')
const grid = grid_mesh([0,1,2], [0,1], (x,y) => vec3(x,y,0))
assert(len(grid.vertices) == 6, 'grid vertex count')
assert(len(grid.triangles) == 4, 'grid triangle count')
'Geometry checks passed'
