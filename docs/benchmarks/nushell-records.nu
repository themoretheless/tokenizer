# JSON array counterpart of record_summary; validates before filtering.
def finite [value: float] {
  if ($value != $value) or (($value | math abs) > 1.7976931348623157e308) {
    error make {msg: 'number must be finite'}
  }
  $value
}
def main [input: path] {
  let rows = (open --raw $input | from json)
  if not (($rows | describe) =~ '^(list|table)') {
    error make {msg: 'Input must be an array'}
  }
  if ($rows | length) > 1000 { error make {msg: 'At most 1000 rows'} }
  let checked = ($rows | each {|row|
    if ($row.group | describe) != 'string' { error make {msg: 'group must be string'} }
    if not (($row.amount | describe) in ['int' 'float']) { error make {msg: 'amount must be numeric'} }
    if ($row.active | describe) != 'bool' { error make {msg: 'active must be boolean'} }
    {group: $row.group, amount: (finite ($row.amount | into float)), active: $row.active}
  } | collect)
  $checked | where active | group-by group | transpose group rows | each {|entry|
    let total = ($entry.rows | get amount | reduce --fold 0.0 {|x, total| finite ($total + $x)})
    {group: $entry.group, total: $total}
  } | collect | to json --raw
}
