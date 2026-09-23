
/obj/state_only
    icon = 'state.dmi'
    icon_state = "off"
/obj/icon_only
    icon = 'old.dmi'
    icon_state = "steady"
/datum/demir/test/bake(atom/target)
    if(istype(target, /obj/state_only))
        target.icon_state = "on"
    else if(istype(target, /obj/icon_only))
        target.icon = 'new.dmi'
