var/global/constructor_count = 0
/turf/staged_floor
/turf/staged_wall
/obj/staged_root
/obj/staged_kept
/obj/staged_module
/area/staged_source
/area/staged_module
/datum/demir/test
    var/source_floor_seen = FALSE
    var/turf/original_turf
/datum/demir/test/New()
    constructor_count += 1
    demir_modular_root(/obj/staged_root)
    world.log << "constructor"
    original_turf = locate(1, 1, 1)
    source_floor_seen = istype(original_turf, /turf/staged_floor)
    var/obj/staged_kept/kept = locate(/obj/staged_kept)
    kept.name = "constructor"
/datum/demir/test/prepare(atom/target)
    world.log << "prepare:[target.type]"
    target.name += ":prepared"
/datum/demir/test/bake(atom/target)
    target.icon_state = "[constructor_count]-[source_floor_seen]-[target.name]-[istype(get_step(target, EAST), /turf/staged_wall)]"
/datum/demir/test/process_modular_root(list/roots)
    for(var/obj/staged_root/root as anything in roots)
        demir_place_modular(list(list("root" = root, "maps" = list("staged.dmm"))))
