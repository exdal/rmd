
/obj/panel
    icon = 'old.dmi'
/datum/demir/test/bake(atom/target)
    flick("opening", target)
    animate(target, alpha = 0, time = 10)
    target.icon = icon(icon('panels.dmi', "on"))
    target.icon_state = "on"
