/datum/demir/test
    default = TRUE

    ui(atom/target)
        var/popup = imgui_mouse_popup()
        imgui_open_popup(popup)
        if(imgui_begin_popup(popup))
            imgui_text("inside")
            imgui_end_popup()
