/datum/airlock
    var/suffix = null
/proc/test()
    var/datum/airlock/airlock = new
    return "closed" + airlock.suffix
