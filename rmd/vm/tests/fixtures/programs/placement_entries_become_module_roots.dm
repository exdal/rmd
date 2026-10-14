/obj/root
	var/key

/obj/connector
/turf/hole
/area/hole

/datum/demir/test

/datum/demir/test/New()
	..()
	demir_modular_root(/obj/root)

/datum/demir/test/process_modular_root(list/roots)
	var/list/placements = list()
	for(var/obj/root/root as anything in roots)
		if(root.key == "none")
			continue

		switch(root.key)
			if("empty")
				placements += list(list("root" = root, "maps" = list()))
			if("full")
				placements += list(list("root" = root, "maps" = list("a.dmm", "b.dmm", "a.dmm"), "anchor" = /obj/connector, "skip" = list(/turf/hole, /area/hole), "unknown" = 1))
			if("single_skip")
				placements += list(list("root" = root, "maps" = list("a.dmm"), "skip" = /turf/hole))
			if("error")
				placements += list(list("root" = root, "error" = "no room in config"))
	demir_place_modular(placements)
