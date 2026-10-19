/area/zone
/turf/floor
/obj/decoy
/obj/wanted

/datum/demir/test/bake(atom/target)
    if(!istype(target, /turf/floor) || target.x != 1)
        return
    var/obj/wanted/declared = locate()
    var/obj/wanted/assigned
    assigned = locate()
    var/untyped = locate()
    target.name = "[declared?.x] [assigned == declared] [isnull(untyped)]"
