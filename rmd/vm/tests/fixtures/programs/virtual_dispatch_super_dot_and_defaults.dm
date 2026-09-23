
/datum/base
    proc/value(a = 3)
        return a * 2
/datum/base/child
    value(a = 4)
        . = ..()
        . += 1
/proc/test()
    var/datum/base/child/object = new
    return object.value()
