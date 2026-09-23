
/obj/panel
    var/glow = 40
/datum/demir/test/ui(atom/target)
    if(!imgui_begin("Panel"))
        imgui_end()
        return
    imgui_text("[target.name]")
    if(imgui_button("Reset"))
        imgui_text("reset")
    var/obj/panel/panel = target
    target.name = "ui writes are rolled back"
    if(imgui_checkbox("lit", 1))
        imgui_slider("glow", panel.glow, 0, 255)
    imgui_end()
