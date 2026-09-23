
/datum/test
    proc/value()
        return 1
/proc/test()
    var/datum/test/object
    return object?.value()
