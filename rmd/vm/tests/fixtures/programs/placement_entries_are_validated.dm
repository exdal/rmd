/obj/root
	var/key

/obj/plain

/obj/connector

/datum/demir/test

/datum/demir/test/New()
	..()
	demir_modular_root(/obj/root)

/datum/demir/test/process_modular_root(list/roots)
	var/obj/root/root = roots[1]
	switch(root.key)
		if("foreign")
			demir_place_modular(list(list("root" = locate(/obj/plain), "maps" = list("a.dmm"))))
		if("no_root")
			demir_place_modular(list(list("maps" = list("a.dmm"))))
		if("duplicate_in_call")
			demir_place_modular(list(list("root" = root, "maps" = list("a.dmm")), list("root" = root, "error" = "again")))
		if("duplicate_across_calls")
			demir_place_modular(list(list("root" = root, "maps" = list("a.dmm"))))
			demir_place_modular(list(list("root" = root, "maps" = list("b.dmm"))))
		if("bad_entry")
			demir_place_modular(list("text"))
		if("bad_maps")
			demir_place_modular(list(list("root" = root, "maps" = "a.dmm")))
		if("bad_map_entry")
			demir_place_modular(list(list("root" = root, "maps" = list(1))))
		if("bad_anchor")
			demir_place_modular(list(list("root" = root, "maps" = list(), "anchor" = "text")))
		if("undefined_anchor")
			demir_place_modular(list(list("root" = root, "maps" = list(), "anchor" = /obj/missing)))
		if("bad_skip")
			demir_place_modular(list(list("root" = root, "maps" = list(), "skip" = list("text"))))
		if("bad_error")
			demir_place_modular(list(list("root" = root, "error" = 5)))
		if("unknown_kind")
			demir_place_modular(list(list("root" = root, "type" = /obj/plain)))
