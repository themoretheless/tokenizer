import curves
assert(curves.cubic(0, 2, 4, 6, 0) == 0, 'cubic start')
assert(curves.cubic(0, 2, 4, 6, 1) == 6, 'cubic end')
assert(curves.quadratic(vec2(0,0), vec2(1,2), vec2(2,0), 0.5) == vec2(1,1), 'quadratic midpoint')
assert(curves.cubic_tangent(0, 2, 4, 6, 0.5) == 6, 'cubic tangent')
'All curve checks passed'
