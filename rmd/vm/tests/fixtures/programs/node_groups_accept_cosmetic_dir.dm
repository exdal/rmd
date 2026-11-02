/obj/pipe
/obj/duct
/obj/cable
/obj/cable/segment

/datum/demir/test
    New()
        ..()
        demir_node_group(/obj/pipe, null, null, null, TRUE)
        demir_node_group(/obj/duct, null)
        demir_node_group(/obj/cable, null, null, null, TRUE)
        demir_node_group(/obj/cable, null, /obj/cable/segment)
