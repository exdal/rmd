/proc/thrower()
    throw "boom"

/proc/assigned_after_a_throwing_call()
    var/value = 1
    try
        thrower()
        value = 2
    catch
        return value
    return -1

/proc/read_after_the_merge()
    var/value = 1
    var/other = 10
    try
        value = 2
        other = 20
        thrower()
        value = 3
    catch
        other += 1
    return value * 100 + other

/proc/assigned_in_a_loop()
    var/total = 0
    try
        for(var/i in 1 to 5)
            total += i
            if(i == 3)
                thrower()
    catch
        return total
    return -1

/proc/rethrown_from_an_inner_catch()
    var/value = 1
    try
        try
            value = 2
            thrower()
        catch(var/error)
            throw error
    catch
        return value
    return -1

/proc/written_by_an_inner_catch()
    var/value = 1
    try
        try
            thrower()
        catch
            value = 5
        thrower()
    catch
        return value
    return -1

/proc/inside_an_immediate_spawn()
    var/list/events = list()
    var/value = 1
    spawn(-1)
        try
            value = 2
            thrower()
        catch
            events += value
    events += value
    return events.Join(",")
