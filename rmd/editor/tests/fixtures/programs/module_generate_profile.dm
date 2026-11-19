/obj/module
    icon_state = "table"

/datum/demir/example
    default = TRUE

/datum/demir/example/generate()
    new /turf/closed/wall(locate(3, 1, 1))
    new /obj/module(locate(4, 1, 1))

/datum/demir/example/bake(atom/target)
    target.name = "baked"
