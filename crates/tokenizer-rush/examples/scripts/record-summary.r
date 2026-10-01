// The host supplies typed tuples: (group, amount, active).
records()
    | filter(row => row[2])
    | fold_by(row => row[0], 0, (total, row) => total + row[1])
    | map(group => {group: group.key, total: group.value})
