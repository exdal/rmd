// Baked by rmd, never compiled by BYOND
#ifdef __DEMIR_BAKE__

/proc/demir_highlights(atom/target)
	if(!istype(target, /obj/structure/table))
		return null

	// A rectangle around the table, and the ring of tiles just outside it.
	var/list/ring = list()
	for(var/offset in -2 to 2)
		ring += list(list(offset, -2), list(offset, 2))
	for(var/offset in -1 to 1)
		ring += list(list(-2, offset), list(2, offset))

	return list(
		list("x" = -1, "y" = -1, "width" = 3, "height" = 3, "label" = "table"),
		list("tiles" = ring, "color" = "#40a0ff", "fill" = 0.08, "when" = DEMIR_HIGHLIGHT_ALWAYS),
	)

/proc/demir_bake(atom/target)
	if(!istype(target, /turf/closed/wall))
		return

	var/junction = 0
	for(var/direction in list(NORTH, SOUTH, EAST, WEST))
		if(istype(get_step(target, direction), /turf/closed/wall))
			junction |= direction

	target.name = "wall [junction]"

#endif
