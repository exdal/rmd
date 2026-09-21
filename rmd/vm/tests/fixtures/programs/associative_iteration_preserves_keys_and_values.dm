
/proc/test()
    var/list/values = list("a" = 2, "b" = 5)
    var/total = 0
    for(var/key, var/value in values)
        total += values[key] + value
    return total
