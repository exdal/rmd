
/obj/lamp
/datum/demir/test/light(atom/target)
    if(istype(target, /obj/lamp))
        target.demir_light_range = 4
        target.demir_light_power = 1
        target.demir_light_color = "#ffffff"
        target.demir_light_height = 0
        target.demir_light_offset_x = 0.5
