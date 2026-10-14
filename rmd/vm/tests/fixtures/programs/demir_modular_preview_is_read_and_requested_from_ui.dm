/datum/demir/test
    default = TRUE

/datum/demir/test/ui(atom/target)
    if(imgui_begin("Panel"))
        if(imgui_checkbox("Flip", 0))
            demir_set_modular_preview(1)
            demir_set_modular_preview(!demir_modular_preview())
    imgui_end()
