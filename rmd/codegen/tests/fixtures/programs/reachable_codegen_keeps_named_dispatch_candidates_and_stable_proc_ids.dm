
/datum/base
    proc/live()
        return 1
    proc/dead()
        return 2
/datum/base/child/live()
    return 3
/proc/entry(datum/base/value)
    return value.live()
/proc/unused()
    return 4
