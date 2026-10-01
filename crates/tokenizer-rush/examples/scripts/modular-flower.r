import curves
range(0, 360, 5)
    | map(a => curves.polar(30 + 10 * cos(deg(a * 6)), deg(a)))
    | polygon
