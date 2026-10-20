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

/datum/demir/example/New()
    ..()
    demir_place_modular(list(list(
    	"at" = list(4, 1, 1),
    	"maps" = list("./a.dmm", "./b.dmm"),
    	"anchor" = /obj/modular_map_connector,
    	"skip" = list(/turf/template_noop, /area/template_noop),
    )))

/datum/demir/example/bake(atom/target)
    target.name = "baked"
