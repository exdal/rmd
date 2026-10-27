/proc/test(a)
    var/kept = a + 1
    spawn(-1)
        var/inside = a + 2
        if(inside)
            inside = a + 3
        world.log << inside
    return kept
