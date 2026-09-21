
/datum/live/New()
    return 1
/datum/dead/New()
    return 2
/proc/entry(kind)
    return new kind
/proc/unrelated()
    return 3
