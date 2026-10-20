/obj/root
	var/key

/datum/demir/from_pass

/datum/demir/from_pass/New()
	..()
	demir_modular_root(/obj/root)

/datum/demir/from_pass/process_modular_root(list/roots)
	demir_place_modular(list(list("at" = list(1, 1, 1), "maps" = list())))

/datum/demir/from_prepare

/datum/demir/from_prepare/prepare(atom/target)
	demir_place_modular(list(list("at" = list(1, 1, 1), "maps" = list())))

/datum/demir/from_ui

/datum/demir/from_ui/ui(atom/target)
	if(imgui_begin("Panel"))
		if(imgui_button("Place"))
			demir_place_modular(list(list("at" = list(1, 1, 1), "maps" = list())))
	imgui_end()
