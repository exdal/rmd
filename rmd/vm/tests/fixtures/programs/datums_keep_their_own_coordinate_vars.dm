
/datum/light
    var/x = 1
    var/y

/proc/test()
    var/datum/light/light = new
    light.x = 4.5
    light.y = 2
    return "[light.x] [light.y]"
