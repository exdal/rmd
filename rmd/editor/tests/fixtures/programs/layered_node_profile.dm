/obj/pipe
    var/piping_layer = 3
/obj/pipe/layer2
    piping_layer = 2
/obj/pipe/layer4
    piping_layer = 4

/datum/demir/example
    default = TRUE

/datum/demir/example/New()
    demir_node_group(/obj/pipe, /turf/closed, null, "piping_layer")
