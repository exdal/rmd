
/proc/helper(n)
    return 1
/datum/thing
    proc/helper(n)
        return 2
    proc/work()
        return helper(0)
/datum/thing/special
    helper(n)
        return 3
/proc/test()
    var/datum/thing/T = new /datum/thing/special
    return T.work()
