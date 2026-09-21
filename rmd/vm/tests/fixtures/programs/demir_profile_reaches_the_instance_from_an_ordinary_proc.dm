
/turf/wall
    icon_state = "static"
    proc/demir_style()
        var/datum/demir/test/profile = demir_profile()
        return profile.style
/datum/demir/test
    var/style

    New()
        ..()
        style = "lit"

    bake(atom/target)
        target.icon_state = target.demir_style()
