// Finite prefixes and short-circuit consumers of a huge numeric source.
let source = range_iter(0, 1000000000000)
let prefix = source | map(x => x * 2) | filter(x => x >= 4) | collect(3)
assert(prefix == [4, 6, 8])
assert(source | any(x => x == 3))
assert(not (source | all(x => x < 3)))
assert((range_iter(0, 5) | fold(0, (sum, x) => sum + x)) == 10)
