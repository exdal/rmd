/proc/thrower()
    throw "late"

/proc/after_break()
    var/list/events = list()
    try
        for(var/i in 1 to 3)
            try
                if(i == 2)
                    break
                events += "body[i]"
            catch
                events += "stale"
        thrower()
    catch(var/error)
        events += "caught [error]"
    return events.Join(",")

/proc/after_continue()
    var/total = 0
    for(var/i in 1 to 4)
        try
            if(i % 2)
                continue
            total += i
        catch
            total = -100
    try
        thrower()
    catch
        total += 1000
    return total

/proc/after_breaking_two_tries()
    var/list/events = list()
    while(1)
        try
            try
                break
            catch
                events += "inner"
        catch
            events += "outer"
    try
        thrower()
    catch(var/error)
        events += "caught [error]"
    return events.Join(",")

/proc/after_forward_goto()
    var/list/events = list()
    try
        try
            goto escaped
        catch
            events += "stale"
        escaped:
            thrower()
    catch(var/error)
        events += "caught [error]"
    return events.Join(",")

/proc/after_backward_goto()
    var/list/events = list()
    var/count = 0
    again:
        count++
        try
            if(count < 3)
                goto again
            events += "body[count]"
        catch
            events += "stale"
    try
        thrower()
    catch(var/error)
        events += "caught [error]"
    return events.Join(",")
