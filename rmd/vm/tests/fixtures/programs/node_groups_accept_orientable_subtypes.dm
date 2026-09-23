/obj/pipe
/obj/pipe/segment
/obj/other
/turf/closed
/obj/grille

/datum/demir/test
    New()
        ..()
        demir_node_group(/obj/pipe, /turf/closed, /obj/pipe/segment)
        demir_node_group(/obj/pipe, /obj/grille)
        demir_node_group(/obj/pipe, /obj/other, /obj/pipe)
        demir_node_group(/obj/pipe, /obj/other, /obj/other)
        demir_node_orientation(/obj/pipe/segment, 1, 3)
        demir_node_orientation(/obj/pipe/segment, 1, 12)
        demir_node_orientation(/obj/other, 4, 12)
        demir_node_orientation(/obj/pipe/segment, 4, 16)
