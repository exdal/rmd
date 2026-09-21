
/obj/runway_light
/datum/demir/test/light(atom/target)
    if(istype(target, /obj/runway_light))
        target.demir_light_range = 0
        target.demir_light_power = 0.5
        target.demir_light_color = "#ffffff"
        target.demir_light_height = 5.76
        target.demir_light_quadratic = 1.1
        target.demir_light_constant = -0.11
