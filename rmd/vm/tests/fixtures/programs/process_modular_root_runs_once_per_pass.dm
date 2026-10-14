/obj/root
	var/key

/obj/root/child

/obj/plain
	var/key

/datum/demir/test
	var/task_calls = 0

/datum/demir/test/New()
	..()
	demir_modular_root(/obj/root)
	demir_modular_root(/obj/root)

/datum/demir/test/process_modular_root(list/roots)
	task_calls += 1
	for(var/obj/root/root as anything in roots)
		demir_place_modular(list(list("root" = root, "maps" = list("[root.key]:[roots.len]:[task_calls]"))))

/datum/demir/batched

/datum/demir/batched/New()
	..()
	demir_modular_root(/obj/root)

/datum/demir/batched/process_modular_root(list/roots)
	var/list/placements = list()
	for(var/obj/root/root as anything in roots)
		placements += list(list("root" = root, "maps" = list("[root.key]:[roots.len]")))
	demir_place_modular(placements)

/datum/demir/no_task

/datum/demir/no_task/New()
	..()
	demir_modular_root(/obj/root)

/datum/demir/crashing

/datum/demir/crashing/New()
	..()
	demir_modular_root(/obj/root)

/datum/demir/crashing/process_modular_root(list/roots)
	demir_place_modular(list(list("root" = roots[1], "maps" = list("a.dmm"))))
	CRASH("task failure")
