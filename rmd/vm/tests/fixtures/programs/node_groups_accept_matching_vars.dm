/obj/cable
/obj/pipe
/obj/bad
/turf/closed

/datum/demir/test
    New()
        ..()
        demir_node_group(/obj/cable, /turf/closed, null, "cable_layer")
        demir_node_group(/obj/pipe, /turf/closed, null, list("piping_layer", "pipe_color", "piping_layer"))
        demir_node_group(/obj/pipe, null, null, list("pipe_color", "hidden"))
        demir_node_group(/obj/bad, null, null, "piping_layer")
        demir_node_group(/obj/bad, null, null, list("pipe_color", 2))
