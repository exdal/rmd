
/proc/test()
    var/list/values = list(1, 2, 3)
    var/total = 0
    for(var/value in values)
        total += value
        values.Add(value + 10)
    return total * 10 + length(values)
