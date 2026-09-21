
/datum/base
/datum/base/child
/datum/other
/proc/test()
    var/datum/base/value = new /datum/other
    return istype(value)
