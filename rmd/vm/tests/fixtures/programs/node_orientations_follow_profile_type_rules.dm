/obj/link
    var/extra = 0
/obj/link/segment
    extra = 4
/obj/link/junction
    extra = 1

/datum/demir/test
    New()
        ..()
        demir_node_group(/obj/link, null, /obj/link/segment)
        for(var/type_path in typesof(/obj/link))
            var/obj/link/link_type = type_path
            var/extra = initial(link_type.extra)
            var/openings = NORTH
            if(extra & 4)
                openings |= SOUTH
            if(extra & 1)
                openings |= turn(NORTH, 90)
            demir_node_orientation(type_path, NORTH, openings)
