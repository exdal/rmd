
/proc/value(a = 5)
    return a
/datum/proc/forwarded(u = 7)
    return u
/datum/child/forwarded(u)
    return ..()
/proc/test()
    var/datum/child/child = new
    return "[value(null)] [child.forwarded()] [child.forwarded(3)]"
