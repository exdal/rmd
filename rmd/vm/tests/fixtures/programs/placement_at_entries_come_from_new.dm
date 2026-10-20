/obj/root
	var/key

/obj/connector
/turf/hole

/datum/demir/test

/datum/demir/test/New()
	..()
	demir_modular_root(/obj/root)
	demir_place_modular(list(
		list("at" = list(5, 2, 1), "maps" = list("a.dmm")),
		list("at" = list(3, 2, 1), "error" = "no room"),
		list("at" = list(9, 1, 2), "maps" = list("b.dmm", "c.dmm"), "anchor" = /obj/connector, "skip" = /turf/hole, "offset" = list(1, -2, 3)),
		list("at" = list(40, 40, 40), "maps" = list()),
	))

/datum/demir/test/process_modular_root(list/roots)
	for(var/obj/root/root as anything in roots)
		demir_place_modular(list(list("root" = root, "maps" = list("root.dmm"))))
