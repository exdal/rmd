/obj/machine
/obj/machine/pump
/obj/machine/pump/fast
/obj/machine/tank

/datum/demir/test
    New()
        ..()
        demir_rotatable(/obj/machine, list(NORTH, SOUTH, EAST, WEST, NORTH, 3, 1.5, "east"))
        demir_rotatable(/obj/machine/pump, SOUTHWEST)
        demir_rotatable(/obj/machine/pump, list(NORTHEAST, SOUTHWEST))
        demir_rotatable(/obj/machine/tank, list(3, 0))
        demir_rotatable("not a type", NORTH)
