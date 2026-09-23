
/turf/styled
    icon_state = "static"
    var/style = 0
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/styled))
        target.icon_state = "[target.style]"
