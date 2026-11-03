/obj/lamp
/datum/demir/test
    light_corner_step = 0.25
/datum/demir/test/light(atom/target)
    if(istype(target, /obj/lamp))
        target.demir_light_range = 3
        target.demir_light_power = 1
        target.demir_light_color = "#ffffff"
        target.demir_light_height = 0
