
/obj/cable
/obj/cable/layered
/obj/pipe

/datum/demir/test/New()
    demir_define_group(1, /obj/cable)

/datum/demir/test/ui(atom/target)
    if(imgui_begin("Panel"))
        if(imgui_button("Redo"))
            demir_rebake(DEMIR_BAKE_APPEARANCE, 1)
    imgui_end()

/datum/demir/test/bake(atom/target)
    target.alpha = 100
