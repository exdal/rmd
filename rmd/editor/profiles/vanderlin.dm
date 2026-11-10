// rmd embeds this profile for the Forced bundled profile setting. To keep it in a Vanderlin
// checkout instead, copy it there and `#include` it from `vanderlin.dme` after `// END_INCLUDE`.
// BYOND skips it because of the guard below.
//
// Smoothing needs the globals and the overlay subsystem stubbed first. Water pipes build their icon
// state from a connection list that the game fills over two init passes.
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

/atom/movable/proc/demir_add_overlay_light()
	if(light_system != MOVABLE_LIGHT || !light_on || !light_outer_range)
		return
	demir_add_light_mask(light_outer_range, light_color)

// The mask /datum/component/overlay_lighting draws, as an underlay instead of vis_contents.
/atom/movable/proc/demir_add_light_mask(outer_range, mask_color)
	var/rounded_range = clamp(CEILING(outer_range, 0.5), 1, 9)
	var/pixel_bounds = ((rounded_range - 1) * 64) + 32
	var/list/light_overlays = /datum/component/overlay_lighting::light_overlays
	var/image/mask = new
	mask.icon = light_overlays["[pixel_bounds]"]
	mask.icon_state = "light2"
	mask.dir = dir
	mask.plane = O_LIGHTING_VISUAL_PLANE
	mask.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
	mask.alpha = 255
	mask.color = mask_color
	mask.pixel_x = -((pixel_bounds - 32) * 0.5)
	mask.pixel_y = mask.pixel_x
	mask.demir_overlay_light = 1
	underlays += mask

// A sconce lights the torch it spawns in Initialize(). The torch is not on a turf, so its light
// component puts the mask on the sconce.
/obj/machinery/light/fueled/torchholder/demir_add_overlay_light()
	if(!ispath(torchy))
		return
	var/obj/item/flashlight/flare/torch/torch = torchy
	demir_add_light_mask(initial(torch.light_outer_range), initial(torch.light_color))

// Fixtures keep the default light vars until update() passes brightness, bulb_power and
// bulb_colour to set_light().
/obj/machinery/light/demir_prepare_light_state()
	light_on = on
	if(!on)
		return
	light_power = bulb_power
	light_color = color ? color : bulb_colour
	light_outer_range = brightness
	if(light_outer_range > 0 && light_outer_range < MINIMUM_USEFUL_LIGHT_RANGE)
		light_outer_range = MINIMUM_USEFUL_LIGHT_RANGE
	if(light_inner_range >= light_outer_range)
		light_inner_range = light_outer_range / 4
	light_falloff_curve = LIGHTING_DEFAULT_FALLOFF_CURVE

// `seton(TRUE)` from Initialize()
/obj/machinery/light/fueled/demir_prepare_light_state()
	on = status == LIGHT_OK
	..()

/obj/machinery/light/fueled/torchholder/demir_prepare_light_state()
	..()
	if(!torchy)
		on = FALSE
		light_on = FALSE

// `lights_on()` from Initialize()
/obj/machinery/light/fueledstreet/demir_prepare_light_state()
	on = TRUE
	..()

// Initialize() sets light_on from the mapped on var or icon_state.
/obj/item/flashlight/demir_prepare_light_state()
	if(icon_state == "[initial(icon_state)]-on")
		on = TRUE
	light_on = on

/obj/item/flashlight/flare/torch/prelit/demir_prepare_light_state()
	on = TRUE
	light_on = TRUE

/obj/item/flashlight/flare/torch/metal/prelit/demir_prepare_light_state()
	on = TRUE
	light_on = TRUE

/obj/item/clothing/head/helmet/leather/shaman_hood/demir_prepare_light_state()
	light_on = on

// Water pipes build their icon state from the `connected` list, keyed by direction, in list order.
// In game each pipe links with its neighbors in Initialize() and machines link back in
// setup_water(). Checking every neighbor from each pipe gives the same result in any order.
/obj/structure/water_pipe/demir_bake_appearance()
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

// GET_TURF_ABOVE() needs SSmapping's z level links, which the editor does not build. Treat the
// next z level as the one above, like the viewer does.
/obj/structure/water_pipe/proc/demir_vertical_turf(offset)
	var/turf/here = get_turf(src)
	if(!here)
		return null

	return locate(here.x, here.y, here.z + offset)

/datum/demir/vanderlin
	default = TRUE
	var/sun_color

	New()
		..()
		if(!GLOB)
			GLOB = new /datum/controller/global_vars/demir_preview
		sun_color = demir_daylight_color()

/atom/proc/demir_prepare_light_state()
	return

/datum/demir/vanderlin/prepare(atom/target)
	target.demir_prepare_smoothing()
	target.demir_prepare_light_state()

/atom/proc/demir_bake_appearance()
	if(smoothing_flags & USES_SMOOTHING)
		smooth_icon()

/atom/proc/demir_finish_appearance()
	return

/atom/movable/demir_finish_appearance()
	demir_add_overlay_light()

// Water pipes neither smooth nor light.
/obj/structure/water_pipe/demir_finish_appearance()
	return

/datum/demir/vanderlin/bake(atom/target)
	target.demir_bake_appearance()
	target.demir_finish_appearance()


// The day cycle picks a random daytime tint. Use the first one so the color stays the same between
// bakes.
/proc/demir_daylight_color()
	var/datum/time_of_day/daytime/noon = /datum/time_of_day/daytime
	var/tint = initial(noon.color)
	if(islist(tint))
		var/list/tints = tint
		return length(tints) ? tints[1] : COLOR_WHITE
	return tint || COLOR_WHITE

// Same as is_sky_visible(), which needs SSmapping's z level links. Treat the next z level as the
// one above, like the water pipes.
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

// Sunlight reaches GLOBAL_LIGHT_RANGE tiles from every turf that sees the sky, and HARD_SUN is the
// -0.5 under the root. Corners take the strongest sunlight instead of the sum, so this is a peak
// source. Each sunlit turf lights its own corners fully, so no separate border pass is needed.
// outdoor_datum.dm undefines both, so their values are repeated here.
/turf/demir_apply_light()
	if(!demir_sees_sky())
		return ..()
	demir_light_range = 3
	demir_light_power = 1
	demir_light_height = -0.5
	var/datum/demir/vanderlin/profile = demir_profile()
	demir_light_color = profile.sun_color
	demir_light_peak = 1

/area/demir_apply_light()
	demir_fullbright = !dynamic_lighting

/datum/demir/vanderlin/light(atom/target)
	target.demir_apply_light()

#endif
