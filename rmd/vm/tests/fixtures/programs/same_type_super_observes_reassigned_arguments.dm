
/datum/test/proc/value(a = 1)
    return a
/datum/test/value(a = 2)
    args[1] += 3
    a += 4
    return ..()
/proc/test()
    var/datum/test/object = new
    return object.value()
