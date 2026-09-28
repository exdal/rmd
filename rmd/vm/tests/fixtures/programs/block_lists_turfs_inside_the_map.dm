
/area/zone
/turf/floor
/proc/describe(list/found)
    var/list/parts = list()
    for(var/turf/entry in found)
        parts += "[entry.x],[entry.y]"
    return jointext(parts, " ")

/datum/demir/test/bake(atom/target)
    if(!istype(target, /turf/floor) || target.x != 1 || target.y != 1)
        return
    var/list/found = list(
        describe(block(0, 0, 1, 2, 2, 1)),
        describe(block(locate(3, 3, 1), locate(2, 1, 1))),
        describe(block(3, 2, 1)),
        describe(block(4, 4, 1, 9, 9, 1)),
    )
    target.name = jointext(found, " | ")
