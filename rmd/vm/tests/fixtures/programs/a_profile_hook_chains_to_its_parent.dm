
/turf/wall
    icon_state = "static"
/datum/demir/base
    default = TRUE
    bake(atom/target)
        target.icon_state = "base"
/datum/demir/base/debug/bake(atom/target)
    ..()
    target.icon_state = "[target.icon_state]-debug"
