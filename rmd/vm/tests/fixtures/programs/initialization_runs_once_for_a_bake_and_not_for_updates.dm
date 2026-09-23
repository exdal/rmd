
var/global/demir_init_count = 0
/turf/initialized
    icon_state = "static"
    var/demir_prepared = 0
/datum/demir/test/New()
    world.log << "hello world"
    demir_init_count += 1
/datum/demir/test/prepare(atom/target)
    if(istype(target, /turf/initialized))
        target.demir_prepared += 1
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/initialized))
        target.icon_state = "[demir_init_count]-[target.demir_prepared]"
