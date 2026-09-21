
/datum/test
    var/value = 4
/proc/test()
    var/datum/test/object = new
    object.value = 9
    return initial(object.value) + issaved(object.value)
