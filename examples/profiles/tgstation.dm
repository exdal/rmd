// A template, not something rmd loads on its own. Copy this file into a tgstation checkout and
// `#include` it from `tgstation.dme`; the guard keeps BYOND compiling it to nothing.
//
// The profile runs the codebase's own smoothing and icon-state code against a map, without
// starting any subsystem. Calling `Initialize(TRUE)` instead faults on nearly every atom, so each
// hook sets up only the data the icon selection it drives actually reads.
#ifdef __DEMIR_BAKE__

// Run the codebase's smoothing setup without starting game subsystems.
/datum/controller/subsystem/mapping/demir_preview/New()
	SSmapping = src

/datum/controller/subsystem/overlays/demir_preview/New()
	SSoverlays = src
	stats = list()

/datum/controller/subsystem/atoms/demir_preview/New()
	SSatoms = src
	initialized = INITIALIZATION_INSSATOMS

/datum/controller/configuration/demir_preview/New()
	config = src

/datum/controller/configuration/demir_preview/Get(entry_type)
	var/datum/config_entry/entry = entry_type
	return initial(entry.default)

/datum/controller/global_vars/demir_preview/New()
	GLOB = src
	if(!SSmapping)
		SSmapping = new /datum/controller/subsystem/mapping/demir_preview
	if(!SSatoms)
		SSatoms = new /datum/controller/subsystem/atoms/demir_preview
	if(!config)
		config = new /datum/controller/configuration/demir_preview
	if(!SSoverlays)
		SSoverlays = new /datum/controller/subsystem/overlays/demir_preview
	bitflag_lists = list()
	gvars_datum_init_order = list()
	InitGlobalcardinals()
	InitGlobaldiagonals()
	InitGlobalalldirs()
	InitGlobaladjacent_direction_lookup()
	InitGlobalpipe_color_name()
	InitGlobalwire_node_generating_types()
	InitGlobalemissive_color()
	InitGlobalem_block_color()

/atom/proc/demir_prepare_smoothing()
	SETUP_SMOOTHING()
	if(uses_integrity)
		atom_integrity = max_integrity

// Overlay lights are pre-baked masks composited on the lighting plane. Recreate the component's
// visual without starting components, signals, or dynamic-luminosity bookkeeping.
/proc/demir_tg_overlay_icon(pixel_bounds)
	switch(pixel_bounds)
		if(32)
			return 'icons/effects/light_overlays/light_32.dmi'
		if(64)
			return 'icons/effects/light_overlays/light_64.dmi'
		if(96)
			return 'icons/effects/light_overlays/light_96.dmi'
		if(128)
			return 'icons/effects/light_overlays/light_128.dmi'
		if(160)
			return 'icons/effects/light_overlays/light_160.dmi'
		if(192)
			return 'icons/effects/light_overlays/light_192.dmi'
		if(224)
			return 'icons/effects/light_overlays/light_224.dmi'
		if(256)
			return 'icons/effects/light_overlays/light_256.dmi'
		if(288)
			return 'icons/effects/light_overlays/light_288.dmi'
		if(320)
			return 'icons/effects/light_overlays/light_320.dmi'
		if(352)
			return 'icons/effects/light_overlays/light_352.dmi'
	return null

/atom/movable/proc/demir_add_overlay_light()
	if(!IS_OVERLAY_LIGHT_SYSTEM(light_system) || !light_on || !light_range || !light_power)
		return

	var/rounded_range = clamp(CEILING(light_range, 0.5), 1, 6)
	var/pixel_bounds = ((rounded_range - 1) * 64) + 32
	var/image/mask = new
	mask.icon = demir_tg_overlay_icon(pixel_bounds)
	mask.icon_state = "light"
	mask.dir = dir
	mask.plane = O_LIGHTING_VISUAL_PLANE
	mask.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
	mask.alpha = min(230, (abs(light_power) * 120) + 30)
	mask.color = light_color
	mask.demir_overlay_light = light_power > 0 ? 1 : -1

	var/offset = (pixel_bounds - 32) * 0.5
	mask.pixel_x = -offset
	mask.pixel_y = -offset

	if(IS_OVERLAY_CONE_LIGHT_SYSTEM(light_system))
		var/cast_range
		if(light_system == OVERLAY_LIGHT_BEAM)
			cast_range = max(round(light_range * 0.5), 1)
		else
			cast_range = clamp(round(light_range * 0.5), 1, 3)
		if(cast_range > 2 && !(ALL_CARDINALS & dir))
			cast_range -= 1

		var/final_distance = cast_range
		var/turf/scanning = get_turf(src)
		for(var/i in 1 to cast_range)
			var/turf/next_turf = get_step(scanning, dir)
			if(isnull(next_turf) || IS_OPAQUE_TURF(next_turf))
				final_distance = i
				break
			scanning = next_turf

		switch(dir)
			if(NORTH)
				mask.pixel_y += 32 * final_distance
			if(SOUTH)
				mask.pixel_y -= 32 * final_distance
			if(EAST)
				mask.pixel_x += 32 * final_distance
			if(WEST)
				mask.pixel_x -= 32 * final_distance

		var/image/cone = new
		cone.icon = 'icons/effects/light_overlays/light_cone.dmi'
		cone.icon_state = "light"
		cone.dir = dir
		cone.plane = O_LIGHTING_VISUAL_PLANE
		cone.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
		cone.alpha = min(120, (abs(light_power) * 60) + 15)
		cone.color = light_color
		cone.pixel_x = -32
		cone.pixel_y = -32
		cone.demir_overlay_light = mask.demir_overlay_light
		underlays += cone

	underlays += mask

// Flashlights normally apply their mapped start_on value during Initialize().
/obj/item/flashlight/proc/demir_prepare_light_state()
	if(start_on)
		light_on = TRUE

/turf/open/floor/light/proc/demir_prepare_light()
	// light_floor.dm undefines these constants before this profile is included:
	// fine = 0, flicker = 1, breaking = 2, broken = 3.
	if(!on || state == 3)
		light_on = FALSE
		light_range = 0
		return
	light_on = TRUE
	light_color = currentcolor
	if(state == 1)
		light_range = 2
	else if(state == 2)
		light_range = 1
	else
		light_range = 3

/image/proc/demir_tag_emissive()
	if(plane == EMISSIVE_PLANE && islist(color))
		var/list/color_matrix = color
		if(color_matrix[17] || color_matrix[18])
			demir_emissive = TRUE
		else if(color_matrix[16] && !color_matrix[19])
			demir_emissive_blocker = TRUE
	for(var/image/underlay in underlays)
		underlay.demir_tag_emissive()
	for(var/image/overlay in overlays)
		overlay.demir_tag_emissive()

/atom/proc/demir_tag_emissive()
	if(plane == EMISSIVE_PLANE && islist(color))
		var/list/color_matrix = color
		if(color_matrix[17] || color_matrix[18])
			demir_emissive = TRUE
		else if(color_matrix[16] && !color_matrix[19])
			demir_emissive_blocker = TRUE
	for(var/image/underlay in underlays)
		underlay.demir_tag_emissive()
	for(var/image/overlay in overlays)
		overlay.demir_tag_emissive()

// Cables connect by layer and store their cardinal links rather than using the
// generic smoothing flags.
/obj/structure/cable/proc/demir_bake_connections()
	linked_dirs = NONE
	for(var/check_dir in GLOB.cardinals)
		if(check_dir & banned_links)
			continue
		var/inverse = REVERSE_DIR(check_dir)
		var/turf/step_turf = get_step(src, check_dir)
		for(var/obj/structure/cable/other_cable in step_turf)
			if(!(other_cable.cable_layer & cable_layer) || (other_cable.banned_links & inverse))
				continue
			linked_dirs |= check_dir
			break
	update_icon_state()
	overlays = update_overlays(UPDATE_OVERLAYS)

// Link this device's nodes with the codebase's own connection checks; atmos_init()
// then runs its native update_appearance() without starting pipe networks.
/obj/machinery/atmospherics/proc/demir_bake_connections()
	// GAGS sheets are generated at runtime, so keep the prebuilt map preview state.
	var/preview_state = icon_state
	var/gags = greyscale_config
	greyscale_config = null
	nodes = list()
	nodes.len = device_type
	atmos_init()
	if(gags)
		icon_state = preview_state

/obj/machinery/atmospherics/pipe/layer_manifold/demir_bake_connections()
	icon_state = "manifoldlayer_center"
	return ..()

/obj/machinery/atmospherics/pipe/multiz/demir_bake_connections()
	icon_state = ""
	center = mutable_appearance(icon, "adapter_center", layer = HIGH_OBJ_LAYER)
	pipe = mutable_appearance(icon, "pipe-[piping_layer]")
	return ..()

// Fluid ducts keep an associative neighbour list and use it to assemble their
// icon-state suffixes. Network construction is unnecessary for their preview.
/obj/machinery/duct/proc/demir_bake_connections()
	neighbours = list()
	for(var/check_dir in GLOB.cardinals)
		var/turf/step_turf = get_step(src, check_dir)
		for(var/obj/machinery/duct/other_duct in step_turf)
			if(!(duct_layer & other_duct.duct_layer))
				continue
			if(duct_color != other_duct.duct_color && duct_color != ATMOS_COLOR_OMNI && other_duct.duct_color != ATMOS_COLOR_OMNI)
				continue
			neighbours[other_duct] = check_dir
			break
	update_icon_state()

// Structure window spawners are mapping helpers. Preview the grille and windows
// their native Initialize() creates without committing those temporary atoms.
/obj/effect/spawner/structure/window/proc/demir_bake_spawned_appearance()
	var/list/nearby_turfs = list(loc)
	for(var/direction in list(NORTH, SOUTH, EAST, WEST, NORTHEAST, NORTHWEST, SOUTHEAST, SOUTHWEST))
		nearby_turfs += get_step(src, direction)

	var/list/all_spawned = list()
	var/list/owned_spawned = list()
	for(var/turf/nearby_turf in nearby_turfs)
		for(var/obj/effect/spawner/structure/window/spawner in nearby_turf)
			var/list/before = list()
			for(var/atom/movable/existing in nearby_turf)
				before += existing
			spawner.Initialize(TRUE)
			for(var/atom/movable/spawned in nearby_turf)
				if(spawned in before)
					continue
				spawned.demir_prepare_smoothing()
				all_spawned += spawned
				if(spawner == src)
					owned_spawned += spawned

	for(var/atom/movable/spawned in all_spawned)
		if(spawned.smoothing_flags & USES_SMOOTHING)
			spawned.smooth_icon()

	icon = null
	icon_state = null
	overlays = list()
	for(var/atom/movable/spawned in owned_spawned)
		var/image/preview = new
		preview.appearance = spawned.appearance
		overlays += preview

/obj/machinery/door/airlock/proc/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	update_appearance(UPDATE_ICON)

/proc/demir_initialize()
	if(!GLOB)
		GLOB = new /datum/controller/global_vars/demir_preview

/proc/demir_prepare(atom/target)
	target.demir_prepare_smoothing()
	if(istype(target, /obj/item/flashlight))
		var/obj/item/flashlight/flashlight = target
		flashlight.demir_prepare_light_state()
	if(istype(target, /turf/open/floor/light))
		var/turf/open/floor/light/light_floor = target
		light_floor.demir_prepare_light()
	// Atmos Initialize() normally derives each port from dir before atmos_init().
	// Smart pipes need those neighboring port directions for can_be_node().
	if(istype(target, /obj/machinery/atmospherics))
		var/obj/machinery/atmospherics/atmos_target = target
		if(atmos_target.pipe_flags & PIPING_CARDINAL_AUTONORMALIZE)
			atmos_target.normalize_cardinal_directions()
		atmos_target.set_init_directions(atmos_target.initialize_directions)

/proc/demir_bake(atom/target)
	if(istype(target, /obj/effect/spawner/structure/window))
		var/obj/effect/spawner/structure/window/spawner = target
		spawner.demir_bake_spawned_appearance()
	else if(istype(target, /obj/structure/cable) && !istype(target, /obj/structure/cable/multilayer))
		var/obj/structure/cable/cable = target
		cable.demir_bake_connections()
	else if(istype(target, /obj/machinery/duct))
		var/obj/machinery/duct/duct = target
		duct.demir_bake_connections()
	else if(istype(target, /obj/machinery/door/airlock))
		var/obj/machinery/door/airlock/airlock = target
		airlock.demir_bake_appearance()
	else if(istype(target, /obj/machinery/atmospherics))
		var/obj/machinery/atmospherics/atmos_target = target
		atmos_target.demir_bake_connections()
	else if(target.smoothing_flags & USES_SMOOTHING)
		target.smooth_icon()
	if(ismovable(target))
		var/atom/movable/movable_target = target
		movable_target.demir_add_overlay_light()
	target.demir_tag_emissive()

/proc/fast_emissive_blocker(atom/target)
	var/mutable_appearance/blocker = new
	blocker.icon = target.icon
	blocker.icon_state = target.icon_state
	blocker.dir = target.dir
	blocker.plane = EMISSIVE_PLANE
	blocker.appearance_flags = target.appearance_flags | EMISSIVE_APPEARANCE_FLAGS
	blocker.demir_emissive_blocker = TRUE
	return blocker

/atom/proc/demir_apply_light()
	if(light_system != COMPLEX_LIGHT || !light_on || !light_range || !light_power)
		return
	demir_light_range = max(light_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_power = light_power
	demir_light_color = light_color
	demir_light_angle = light_angle
	demir_light_dir = light_dir
	demir_light_height = light_height

// Fixtures ship with `on = FALSE` and are switched on by area power during
// post_machine_initialize(), which reaches reagents, SSmachines and prob().
/obj/machinery/light/demir_apply_light()
	if(status == LIGHT_OK)
		on = TRUE
		light_range = brightness
		light_power = bulb_power
		light_color = color || bulb_colour
		light_dir = REVERSE_DIR(dir)
	return ..()

// Space is a fullbright area, so its tiles never hold a lighting object and only the ones
// touching a statically lit turf call enable_starlight(). The values are the ones GLOB.starlight_*
// reads off this type.
/turf/open/space
	demir_light_range = 2
	demir_light_power = 1
	demir_light_color = COLOR_STARLIGHT
	demir_light_edge_only = 1

/area/demir_apply_light()
	demir_fullbright = !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/proc/demir_light(atom/target)
	target.demir_apply_light()

// A border object only blocks the side it stands on, and the lighting corners care about
// whether the whole tile is concealed. IS_OPAQUE_TURF wants ALL_CARDINALS.
/atom/movable/demir_apply_light()
	if(opacity && (flags_1 & ON_BORDER_1))
		demir_blocks_light = 0
	return ..()

#endif
