
/turf/export_test
    icon_state = "static"
    overlays = list()
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/export_test))
        var/image/previous = new
        for(var/i = 1 to 24)
            var/image/next = new
            next.overlays = list(previous, previous)
            previous = next
        target.overlays += previous
