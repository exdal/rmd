
/datum/panel_state
    var/mode = 0

/obj/pipe
    alpha = 255

var/global/datum/panel_state/panel

/datum/demir/test/New()
    panel = new /datum/panel_state

/datum/demir/test/ui(atom/target)
    if(!imgui_begin("Panel"))
        imgui_end()
        return
    if(imgui_radio("Hidden", panel.mode == 0))
        panel.mode = 0
    if(imgui_radio("Shown", panel.mode == 1))
        panel.mode = 1
        demir_rebake(DEMIR_BAKE_APPEARANCE)
    imgui_end()

/datum/demir/test/bake(atom/target)
    target.alpha = panel.mode ? 128 : 0
