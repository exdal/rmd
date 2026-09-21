
/obj/fixture
    pixel_y = 21
/obj/wall
    opacity = 1
/datum/demir/test/light(atom/target)
    if(istype(target, /obj/fixture))
        target.demir_light_range = 3
        target.demir_light_power = 1.6
        target.demir_light_color = "#ffffff"
        target.demir_light_height = 5.76
        target.demir_light_quadratic = 3.52
        target.demir_light_constant = -0.11
