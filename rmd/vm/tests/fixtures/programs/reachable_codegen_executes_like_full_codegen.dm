
/datum/base
    var/value = initialize_value()
    proc/read()
        world.log << "read [value]"
        return value
/datum/base/child/read()
    return ..() + 1
/proc/initialize_value()
    return 6
/proc/entry()
    var/datum/base/child/value = new
    return value.read()
/proc/unreachable()
    return 99
