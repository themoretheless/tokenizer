fn close(a, b) { return (a-b)*(a-b) < 0.000000000001 }
assert(close(length(normalize(vec3(3,4,0))), 1), 'unit vector')
assert(dot(vec3(1,0,0), vec3(0,1,0)) == 0, 'orthogonal dot')
assert(cross(vec3(1,0,0), vec3(0,1,0)) == vec3(0,0,1), 'right handed cross')
assert(close(sin(degrees(90)), 1), 'angle conversion')
assert(smoothstep(0, 1, 0.5) == 0.5, 'smoothstep midpoint')
assert(lerp(vec2(0,0), vec2(2,4), 0.5) == vec2(1,2), 'vector interpolation')
assert(transform_point(translation(vec3(1,2,3)), vec3(0,0,0)) == vec3(1,2,3), 'translation')
'Numeric checks passed'
