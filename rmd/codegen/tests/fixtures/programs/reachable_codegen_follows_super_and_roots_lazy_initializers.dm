
/datum/base/proc/step()
    return 1
/datum/base/child/step()
    return ..()
/datum/base/child/step()
    return ..()
/datum/holder
    var/value = initialize()
/proc/initialize()
    return 3
/proc/entry()
    var/datum/base/child/value = new
    var/datum/holder/holder = new
    return value.step() + holder.value
/proc/unused()
    return 4
