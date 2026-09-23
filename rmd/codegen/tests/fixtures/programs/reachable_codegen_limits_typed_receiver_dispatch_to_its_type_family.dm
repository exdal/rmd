
/datum/wanted/proc/Initialize()
    return 1
/datum/wanted/child/Initialize()
    return 2
/datum/unrelated/proc/Initialize()
    return 3
/proc/entry(values)
    for(var/datum/wanted/value in values)
        value.Initialize()
