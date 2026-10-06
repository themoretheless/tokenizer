const twice = factor => x => x * factor
assert(twice(2)(21) == 42, 'lexical closure')
assert(([1,2] | flat_map(x => [x, x * 10])) == [1,10,2,20], 'flat map ordering')
assert(([1,2,3] | filter(x => x > 1) | fold(0, (sum,x) => sum+x)) == 5, 'pipeline fold')
assert(all([1,2,3], x => x > 0), 'all predicate')
assert(not any([], x => true), 'empty any')
assert(get({x:null}, 'x') == Some(null), 'present null')
assert(get({}, 'x') == None(), 'missing key')
assert(len('Привет🙂') == 7, 'Unicode scalar count')
'Functional checks passed'

// Tuple and record parameters bind directly in higher-order functions.
let points = zip([1, 2], [10, 20]) | map(((x, y)) => vec2(x, y))
assert(points == [vec2(1, 10), vec2(2, 20)])
let sum_point = ({point: (x, y)}) => x + y
assert(sum_point({point: (3, 4), label: "sample"}) == 7)
