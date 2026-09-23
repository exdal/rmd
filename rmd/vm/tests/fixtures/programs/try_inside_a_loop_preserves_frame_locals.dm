
/proc/test()
    var/list/values = list(1, 2, 3)
    var/total = 0
    for(var/value in values)
        var/current = value
        try
            total += current
        catch
            total = -100
        current = 100
    return total
