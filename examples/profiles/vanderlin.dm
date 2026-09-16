// A template, not something rmd loads on its own. Copy this file into a Vanderlin checkout and
// `#include` it from `vanderlin.dme`; the guard keeps BYOND compiling it to nothing.
//
// Vanderlin needs its globals and overlay subsystem stubbed before anything smooths, and its fluid
// pipes assemble an icon state from a connection list that normally fills in over two init passes.
#ifdef __DEMIR_BAKE__

/datum/controller/subsystem/overlays/demir_preview/New()
	SSoverlays = src
	stats = list()

/datum/controller/global_vars/demir_preview/New()
	GLOB = src
	if(!SSoverlays)
		SSoverlays = new /datum/controller/subsystem/overlays/demir_preview
	bitflag_lists = list()
	gvars_datum_init_order = list()
	InitGlobalcardinals()

/atom/proc/demir_prepare_smoothing()
	SETUP_SMOOTHING()

// Fluid pipes keep an associative list of connected directions keyed by the
// direction number, and assemble their icon state from those keys in list
// order. Each pipe links its neighbours pairwise from Initialize(), and
// adjacent machines link back from setup_water(). Baking every pipe from its
// own point of view reaches the same result without a load order.
/obj/structure/water_pipe/proc/demir_bake_connections()
	for(var/key in connected)
		connected[key] = 0

	for(var/direction in GLOB.cardinals)
		var/turf/cardinal_turf = get_step(src, direction)
		if(!cardinal_turf)
			continue

		var/connects = (locate(/obj/structure/water_pipe) in cardinal_turf)
		if(!connects)
			for(var/obj/structure/structure in cardinal_turf)
				if(!structure.accepts_water_input)
					continue
				if(!structure.valid_water_connection(direction, src))
					continue
				connects = TRUE
				break

		if(connects)
			connected["[direction]"] = 1

	if(locate(/obj/structure/water_pipe) in demir_vertical_turf(1))
		connected["[UP]"] = 1
	if(locate(/obj/structure/water_pipe) in demir_vertical_turf(-1))
		connected["[DOWN]"] = 1

	update_appearance(UPDATE_OVERLAYS)

// `GET_TURF_ABOVE()` reads the mapping subsystem's z level links, which the
// editor never builds. Adjacent z levels stack instead, the way the viewer
// draws its underlays.
/obj/structure/water_pipe/proc/demir_vertical_turf(offset)
	var/turf/here = get_turf(src)
	if(!here)
		return null

	return locate(here.x, here.y, here.z + offset)

/proc/demir_initialize()
	if(!GLOB)
		GLOB = new /datum/controller/global_vars/demir_preview
	if(!GLOB.demir_sun_color)
		GLOB.demir_sun_color = demir_daylight_color()

/proc/demir_prepare(atom/target)
	target.demir_prepare_smoothing()

/proc/demir_bake(atom/target)
	if(istype(target, /obj/structure/water_pipe))
		var/obj/structure/water_pipe/pipe = target
		pipe.demir_bake_connections()
		return
	if(target.smoothing_flags & USES_SMOOTHING)
		target.smooth_icon()


/datum/controller/global_vars/demir_preview
	var/demir_sun_color

// The day cycle picks one of the daytime tints at random. Noon is what a mapper wants to see, and
// a fixed choice keeps the view from shimmering between bakes.
/proc/demir_daylight_color()
	var/datum/time_of_day/daytime/noon = /datum/time_of_day/daytime
	var/tint = initial(noon.color)
	if(islist(tint))
		var/list/tints = tint
		return length(tints) ? tints[1] : COLOR_WHITE
	return tint || COLOR_WHITE

// `is_sky_visible()` walks the z stack through the mapping subsystem's level links and a
// transparency trait, neither of which the editor builds. Adjacent z levels stack instead, the way
// the water pipes above already assume.
/turf/proc/demir_sees_sky()
	if(pseudo_roof)
		return FALSE
	var/turf/above = locate(x, y, z + 1)
	if(above)
		return above.demir_sky_passes()
	var/area/here = get_area(src)
	return here && here.outdoors

/turf/proc/demir_sky_passes()
	return FALSE

/turf/open/openspace/demir_sky_passes()
	for(var/obj/structure/thing in src)
		if(thing.weatherproof)
			return FALSE
	return demir_sees_sky()

/atom/proc/demir_apply_light()
	if(light_system != STATIC_LIGHT || !light_on || !light_outer_range || !light_power)
		return
	demir_light_range = max(light_outer_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_inner_range = light_inner_range
	demir_light_curve = light_falloff_curve
	// APPLY_CORNER squares the power and puts the sign back afterwards.
	demir_light_power = (light_power ** 2) * (light_power < 0 ? -1 : 1)
	demir_light_color = light_color
	// LUM_FALLOFF measures flat distance, with no pseudo z term under the root.
	demir_light_height = 0

// Sunlight spreads GLOBAL_LIGHT_RANGE tiles from every turf that can see the sky, and HARD_SUN is
// the -0.5 under the root. Corners keep the closest source rather than the sum, so this is a peak
// source. A lit turf reaches its own corners at full strength, which is what makes the outdoors
// read as daylight without an explicit border pass.
/turf/demir_apply_light()
	if(!demir_sees_sky())
		return ..()
	demir_light_range = 3
	demir_light_power = 1
	demir_light_height = -0.5
	demir_light_color = GLOB.demir_sun_color
	demir_light_peak = 1

/area/demir_apply_light()
	demir_fullbright = !dynamic_lighting

/proc/demir_light(atom/target)
	target.demir_apply_light()

#endif
