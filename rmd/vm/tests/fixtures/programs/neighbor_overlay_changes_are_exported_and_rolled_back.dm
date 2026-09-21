
/turf/overlay_test
    var/list/overlays = list()
    proc/Initialize(mapload)
        overlays = list("base")
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/overlay_test))
        target.Initialize(TRUE)
        var/turf/east = get_step(target, 4)
        if(east)
            east.overlays.Add("edge")
