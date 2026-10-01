# Fixed N=1000, 100 iterations; startup and parsing excluded.
let workloads = [
  {name: closure, expected: 42.0, action: {
    let scale = {|factor| {|x| $x * $factor} }
    let twice = do $scale 2.0
    do $twice 21.0
  }}
  {name: collections, expected: 333666.0, action: {
    let values = (0..<1000 | each {|x| $x | into float} | collect)
    let mapped = ($values | each {|x| $x * 2.0} | collect)
    let filtered = ($mapped | where {|x| $x mod 3.0 == 0.0} | collect)
    $filtered | reduce --fold 0.0 {|x, total| $total + $x}
  }}
  {name: collections_lazy, expected: 333666.0, action: {
    0..<1000 | each {|x| $x | into float} | each {|x| $x * 2.0} |
      where {|x| $x mod 3.0 == 0.0} | reduce --fold 0.0 {|x, total| $total + $x}
  }}
]
print $"Nushell (version | get version); N=1000; no execution budget; timeit loop includes do/ignore overhead"
print "workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us"
for workload in $workloads {
  let actual = do $workload.action
  if $actual != $workload.expected { error make {msg: $"Unexpected result: ($actual)"} }
  for _ in 0..<10 { do $workload.action | ignore }
  let samples = (0..<7 | each {||
    let elapsed = (timeit { for _ in 0..<100 { do $workload.action | ignore } })
    ($elapsed | into int) / 100000.0
  } | sort)
  print ([$workload.name prepared_run 100 ($samples | first) ($samples | get 3) ($samples | last)] | str join "\t")
}
