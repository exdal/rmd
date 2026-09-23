
/datum/live/New()
    return live_helper()
/datum/dead/New()
    return dead_helper()
/proc/live_helper()
    return 1
/proc/dead_helper()
    return 2
/proc/entry()
    return new /datum/live
