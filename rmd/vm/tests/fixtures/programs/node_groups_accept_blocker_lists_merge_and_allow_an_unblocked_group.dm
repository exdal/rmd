
/obj/cable
/obj/pipe
/obj/grille
/obj/window
/obj/bad
/turf/closed

/datum/demir/test
    New()
        ..()
        demir_node_group(/obj/cable, list(/turf/closed, /obj/grille, /obj/window, /turf/closed))
        demir_node_group(/obj/cable, /obj/grille)
        demir_node_group(/obj/cable, /turf/closed)
        demir_node_group(/obj/pipe, null)
        demir_node_group(/obj/bad, /obj/grille)
        demir_node_group(/obj/bad, list(/turf/closed, "not a type"))
