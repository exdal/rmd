/turf/wall
    icon_state = "static"
/datum/demir/test
    var/from_new

    New()
        ..()
        from_new = map_path_text()

    bake(atom/target)
        target.icon_state = "[from_new]|[map_path_text()]"

    proc/map_path_text()
        var/path = demir_map_path()
        if(isnull(path))
            return "null"

        return path
