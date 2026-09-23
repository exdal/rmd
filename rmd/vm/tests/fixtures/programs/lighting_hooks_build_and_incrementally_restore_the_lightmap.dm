
/obj/lamp
/obj/ambient
/datum/demir/test/light(atom/target)
    if(istype(target, /obj/lamp))
        target.demir_light_range = 3
        target.demir_light_power = 1
        target.demir_light_color = "#ff4000"
        target.demir_light_height = 0
    if(istype(target, /obj/ambient))
        target.demir_ambient_color = "#0020ff"
        target.demir_ambient_power = 0.25
