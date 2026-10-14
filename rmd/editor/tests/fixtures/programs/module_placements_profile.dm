/obj/modular_map_root
    icon_state = "table"
    var/key = "outer"

/obj/modular_map_connector

/turf/template_noop

/area/template_noop

/obj/module
    icon_state = "table"

/datum/demir/example
    default = TRUE

/datum/demir/example/proc/place(atom/target)
    demir_place_modular(list(list(
        "root" = target,
        "maps" = list("./a.dmm"),
        "anchor" = /obj/modular_map_connector,
        "skip" = list(/turf/template_noop, /area/template_noop),
    )))

/datum/demir/example/prepare(atom/target)
    var/obj/modular_map_root/root = target
    if(istype(root) && root.key == "prepared")
        place(root)

/datum/demir/example/bake(atom/target)
    target.name = "baked"

/datum/demir/example/ui(atom/target)
    if(imgui_begin("Panel"))
        if(imgui_button("Place") && target)
            place(target)
    imgui_end()
