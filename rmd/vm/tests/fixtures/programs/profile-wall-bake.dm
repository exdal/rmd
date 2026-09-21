
/turf/wall
    icon_state = "static"
/datum/demir/test/bake(atom/target)
    if(!istype(target, /turf/wall))
        return
    var/junction = 0
    for(var/direction in list(1, 2, 4, 8))
        if(istype(get_step(target, direction), /turf/wall))
            junction |= direction
    target.icon_state = "wall-[junction]"
