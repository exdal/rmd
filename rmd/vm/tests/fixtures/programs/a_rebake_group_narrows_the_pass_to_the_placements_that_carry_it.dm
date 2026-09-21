
/datum/panel_state
    var/tint = 0

/obj/cable
/obj/pipe

var/global/datum/panel_state/panel

/datum/demir/test/New()
    panel = new /datum/panel_state
    demir_define_group(1, /obj/cable)
    demir_define_group(2, /obj/pipe)

/datum/demir/test/ui(atom/target)
    if(!imgui_begin("Panel"))
        imgui_end()
        return
    var/tint = imgui_slider("tint", panel.tint, 0, 255)
    if(tint != panel.tint)
        panel.tint = tint
        demir_rebake(DEMIR_BAKE_APPEARANCE, 1)
    imgui_end()

/datum/demir/test/bake(atom/target)
    target.alpha = panel.tint
