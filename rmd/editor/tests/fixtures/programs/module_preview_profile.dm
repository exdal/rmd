/obj/modular_map_root
    icon_state = "table"
    var/config_file = "config.toml"
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
    demir_modular_root(/obj/modular_map_root)

/datum/demir/example/bake(atom/target)
    target.name = "baked"

/datum/demir/example/ui(atom/target)
    if(imgui_begin("Panel"))
        var/previewed = imgui_checkbox("Preview modules", demir_modular_preview())
        if(previewed != demir_modular_preview())
            demir_set_modular_preview(previewed)
    imgui_end()

/datum/demir/example/process_modular_root(list/roots)
    var/list/configs = list()
    var/list/placements = list()
    for(var/obj/modular_map_root/root as anything in roots)
        var/path = root.config_file
        if(!(path in configs))
            configs[path] = read_module_config(path)

        var/list/config = configs[path]
        if(istext(config))
            placements += list(list("root" = root, "error" = config))
            continue

        var/list/rooms = config["rooms"]
        var/list/room = rooms && rooms[root.key]
        if(!room)
            placements += list(list("root" = root, "error" = "no room [root.key] in [path]"))
            continue

        var/list/files = list()
        for(var/file in room["modules"])
            files += config["directory"] + file

        placements += list(list(
            "root" = root,
            "maps" = files,
            "anchor" = /obj/modular_map_connector,
            "skip" = list(/turf/template_noop, /area/template_noop),
        ))
    demir_place_modular(placements)

/datum/demir/example/proc/read_module_config(path)
    var/text = file2text(path)
    if(!text)
        return "cannot read [path]"

    try
        return demir_toml_decode(text)
    catch(var/exception/error)
        return "invalid [path]: [error.name]"
