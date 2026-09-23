
/turf/wall
    icon_state = "static"
    var/smooth = 1
/datum/demir/test/bake(atom/target)
    var/junction = 0
    for(var/direction in list(1, 2, 4, 8))
        var/turf/wall/neighbour = get_step(target, direction)
        if(istype(neighbour) && neighbour.smooth)
            junction |= direction
    target.icon_state = "[junction]"
