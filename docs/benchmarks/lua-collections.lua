-- Benchmark support library: ordinary Lua, loaded once before measurements.
function range_values(n)
    local values = {}
    for i = 0, n - 1 do values[#values + 1] = i + 0.0 end
    return values
end
function map_values(values, callback)
    local result = {}
    for i = 1, #values do result[i] = callback(values[i]) end
    return result
end
function filter_values(values, callback)
    local result = {}
    for i = 1, #values do
        if callback(values[i]) then result[#result + 1] = values[i] end
    end
    return result
end
function fold_values(values, initial, callback)
    local result = initial
    for i = 1, #values do result = callback(result, values[i]) end
    return result
end

-- A nil result terminates these numeric iterators.
function range_iterator(n)
    local index = 0
    return function()
        if index >= n then return nil end
        local value = index + 0.0
        index = index + 1
        return value
    end
end
function map_iterator(source, callback)
    return function()
        local value = source()
        if value == nil then return nil end
        return callback(value)
    end
end
function filter_iterator(source, callback)
    return function()
        while true do
            local value = source()
            if value == nil then return nil end
            if callback(value) then return value end
        end
    end
end
function fold_iterator(source, initial, callback)
    local result = initial
    while true do
        local value = source()
        if value == nil then return result end
        result = callback(result, value)
    end
end
