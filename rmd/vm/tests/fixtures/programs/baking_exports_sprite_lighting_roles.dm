
/obj/light
/datum/demir/test/bake(atom/target)
    if(!istype(target, /obj/light))
        return
    target.demir_emissive = TRUE
    var/image/glow = new
    glow.icon_state = "glow"
    glow.demir_emissive = TRUE
    var/image/blocker = new
    blocker.icon_state = "blocker"
    blocker.demir_emissive_blocker = TRUE
    glow.overlays += blocker
    var/image/overlay_light = new
    overlay_light.icon_state = "overlay-light"
    overlay_light.demir_overlay_light = 1
    glow.overlays += overlay_light
    var/image/darkness = new
    darkness.icon_state = "darkness"
    darkness.demir_overlay_light = -1
    glow.overlays += darkness
    target.overlays += glow
