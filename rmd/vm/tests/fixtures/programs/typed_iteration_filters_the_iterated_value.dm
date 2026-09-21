
/datum/base
/datum/base/wanted
/datum/other
/proc/test()
    var/list/values = list(new /datum/base/wanted, new /datum/other, new /datum/base/wanted)
    var/count = 0
    for(var/datum/base/value in values)
        count++
    return count
