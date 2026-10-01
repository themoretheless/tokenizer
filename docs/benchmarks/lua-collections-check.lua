-- Verify demand-driven evaluation independently of the timed workloads.
local reads, maps, predicates = 0, 0, 0
local source = range_iterator(10)
local counted = function()
    reads = reads + 1
    return source()
end
local mapped = map_iterator(counted, function(x)
    maps = maps + 1
    return x * 2.0
end)
local filtered = filter_iterator(mapped, function(x)
    predicates = predicates + 1
    return x % 3.0 == 0.0
end)
assert(reads == 0 and maps == 0 and predicates == 0)
assert(filtered() == 0.0)
assert(reads == 1 and maps == 1 and predicates == 1)
assert(filtered() == 6.0)
assert(reads == 4 and maps == 4 and predicates == 4)
assert(fold_iterator(filtered, 0.0, function(a,b) return a+b end) == 30.0)
assert(reads == 11 and maps == 10 and predicates == 10)
assert(range_iterator(0)() == nil)
assert(fold_iterator(range_iterator(0), 7.0, function() error('empty') end) == 7.0)
