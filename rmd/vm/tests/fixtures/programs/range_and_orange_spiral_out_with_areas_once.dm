
/area/zone
/turf/floor
/obj/thing
/proc/describe(list/found)
    var/list/parts = list()
    for(var/atom/entry in found)
        if(isturf(entry))
            parts += "[entry.x],[entry.y]"
        else if(isarea(entry))
            parts += "A"
        else
            parts += "O"
    return jointext(parts, " ")

/datum/demir/test/bake(atom/target)
    if(!istype(target, /turf/floor) || target.x != 2 || target.y != 2)
        return
    target.name = "[describe(orange(1, target))] | [describe(range(target, 1))]"
