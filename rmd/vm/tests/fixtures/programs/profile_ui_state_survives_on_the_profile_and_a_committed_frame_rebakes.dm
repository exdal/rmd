
/obj/lamp

/datum/demir/test
    var/lit = 1

    New()
        ..()
        demir_define_group(2, /obj/lamp)

    ui(atom/target)
        if(!imgui_begin("Panel"))
            imgui_end()
            return
        var/checked = imgui_checkbox("lit", src.lit)
        if(checked != src.lit)
            src.lit = checked
            demir_rebake(DEMIR_BAKE_APPEARANCE | DEMIR_BAKE_LIGHT, 2)
        imgui_end()

    light(atom/target)
        if(src.lit)
            target.demir_light_range = 3
            target.demir_light_power = 1

    bake(atom/target)
        target.icon_state = src.lit ? "on" : "off"
