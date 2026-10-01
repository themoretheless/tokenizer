local totals, order = {}, {}
for row in records do
    if row[3] then
        local group, amount = row[1], row[2]
        if totals[group] == nil then
            order[#order + 1] = group
            totals[group] = 0
        end
        local next = totals[group] + amount
        if next ~= next or next == math.huge or next == -math.huge then
            error("total must be finite")
        end
        totals[group] = next
    end
end
local result = {}
for i, group in ipairs(order) do
    result[i] = {group = group, total = totals[group]}
end
return result
