// Baked by rmd, never compiled by BYOND
#ifdef __DEMIR_BAKE__

/proc/demir_bake(atom/target)
	if(!istype(target, /turf/closed/wall))
		return

	var/junction = 0
	for(var/direction in list(NORTH, SOUTH, EAST, WEST))
		if(istype(get_step(target, direction), /turf/closed/wall))
			junction |= direction

	target.name = "wall [junction]"

#endif
