
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/datum/base/proc/live()
    return 1
/datum/base/proc/dead()
    return 2
/proc/entry()
    var/datum/base/value = new /datum/base
    return call(value, "live")()
