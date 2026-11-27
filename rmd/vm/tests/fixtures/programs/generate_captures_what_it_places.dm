/turf/a
/turf/b
/obj/rock
/obj/tree

/datum/demir/terrain

/datum/demir/terrain/generate()
	var/turf/replaced = new /turf/b(locate(1, 1, 1))

	var/turf/covered = new /turf/b(locate(2, 1, 1))
	new /obj/rock(covered)
	new /obj/tree(covered)

	var/turf/open = locate(3, 1, 1)
	var/obj/rock/host = new /obj/rock(open)
	new /obj/tree(host)

	var/obj/tree/gone = new /obj/tree(locate(4, 1, 1))
	del(gone)

/datum/demir/faulting

/datum/demir/faulting/generate()
	new /turf/b(locate(1, 1, 1))
	var/zero = 0
	return 1 / zero

/datum/demir/plain

/datum/demir/looping

/datum/demir/looping/generate()
	var/count = 0
	for(var/i in 1 to 30000)
		count += 1
	new /obj/rock(locate(1, 1, 1))
