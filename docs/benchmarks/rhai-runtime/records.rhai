let totals = #{};
let order = [];
loop {
    let row = next_record();
    if row == () { break; }
    if row[2] {
        let key = row[0];
        if !totals.contains(key) {
            totals[key] = 0.0;
            order.push(key);
        }
        let next = totals[key] + row[1];
        if !finite(next) { throw "total must be finite"; }
        totals[key] = next;
    }
}
let result = [];
for key in order {
    result.push(#{group: key, total: totals[key]});
}
result
