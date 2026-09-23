
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/datum/base/proc/live()
    return 1
/datum/base/proc/dead()
    return 2
/proc/entry(target)
    return call(target, "live")()
/proc/unrelated()
    return 3
