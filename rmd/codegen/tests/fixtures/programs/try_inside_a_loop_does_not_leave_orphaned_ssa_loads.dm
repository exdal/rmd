
/proc/test(values)
    var/total = 0
    for(var/value in values)
        try
            total += value
        catch
            total = -1
    return total
