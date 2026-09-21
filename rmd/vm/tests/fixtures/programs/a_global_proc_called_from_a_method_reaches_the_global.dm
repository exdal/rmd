
/proc/helper(n)
    return n * 2
/datum/thing
    proc/work()
        return helper(21)
/proc/test()
    var/datum/thing/T = new
    return T.work()
