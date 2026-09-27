
/obj/moved
    transform = matrix(1, 0, -4, 0, 1, -4)
/obj/turned
/obj/plain
/datum/demir/test/bake(atom/target)
    if(istype(target, /obj/turned))
        target.transform = matrix(90, MATRIX_ROTATE)
