/obj/thing
	var/key

/datum/demir/from_new

/datum/demir/from_new/New()
	..()
	var/obj/thing/target = locate(/obj/thing)
	demir_place_modular(list(list("root" = target, "maps" = list("new.dmm"))))

/datum/demir/from_prepare

/datum/demir/from_prepare/prepare(atom/target)
	demir_place_modular(list(list("root" = target, "maps" = list("prepare.dmm"))))

/datum/demir/from_bake

/datum/demir/from_bake/bake(atom/target)
	demir_place_modular(list(list("root" = target, "maps" = list("bake.dmm"))))

/datum/demir/from_highlights

/datum/demir/from_highlights/highlights(atom/target)
	demir_place_modular(list(list("root" = target, "maps" = list("highlights.dmm"))))
	return list()

/datum/demir/replacing

/datum/demir/replacing/New()
	..()
	var/obj/thing/target = locate(/obj/thing)
	demir_place_modular(list(
		list("root" = target, "maps" = list("first.dmm")),
		list("root" = target, "error" = "second"),
	))

/datum/demir/replacing/prepare(atom/target)
	demir_place_modular(list(list("root" = target, "maps" = list("third.dmm"))))

/datum/demir/unplaced

/datum/demir/unplaced/prepare(atom/target)
	var/obj/thing/made = new /obj/thing()
	demir_place_modular(list(list("root" = made, "maps" = list("made.dmm"))))

/datum/demir/from_ui/ui(atom/target)
	if(imgui_begin("Panel"))
		if(imgui_button("Place"))
			demir_place_modular(list(list("root" = target, "maps" = list("ui.dmm"))))
	imgui_end()

/datum/demir/conditional_prepare

/datum/demir/conditional_prepare/prepare(atom/target)
	var/obj/thing/thing = target
	if(thing.key == "place")
		demir_place_modular(list(list("root" = target, "maps" = list("prepare.dmm"))))

/datum/demir/ui_on_a_root

/datum/demir/ui_on_a_root/New()
	..()
	demir_modular_root(/obj/thing)

/datum/demir/ui_on_a_root/process_modular_root(list/roots)
	for(var/obj/thing/thing as anything in roots)
		if(thing.key == "pass")
			demir_place_modular(list(list("root" = thing, "maps" = list("pass.dmm"))))

/datum/demir/ui_on_a_root/ui(atom/target)
	if(imgui_begin("Panel"))
		if(imgui_button("Place"))
			demir_place_modular(list(list("root" = target, "maps" = list("ui.dmm"))))
	imgui_end()
