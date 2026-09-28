/obj/machine
/datum/demir/test/bake(atom/target)
    var/list/kept = list("floor")
    target.underlays = kept
    kept += "grate"
    target.overlays = "stale"
    target.overlays = null
    if(target.overlays)
        target.overlays += "edge"
