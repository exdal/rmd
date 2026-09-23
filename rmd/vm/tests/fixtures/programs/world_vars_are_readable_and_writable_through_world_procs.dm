
/world
    var/booted = 0
    proc/boot()
        booted = 7
/proc/test()
    world.boot()
    return world.booted
