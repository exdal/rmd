// rmd embeds this integration for the Forced bundled profile setting. To make the profile part of
// a tgstation checkout instead, copy it there and `#include` it from `tgstation.dme`, the guard
// keeps BYOND compiling it to nothing.
//
// The profile runs the codebase's own smoothing and icon-state code against a map, without
// starting any subsystem. Calling `Initialize(TRUE)` instead faults on nearly every atom, so each
// hook sets up only the data the icon selection it drives actually reads.
#ifdef __DEMIR_BAKE__

// What each panel option drives, so a change re-derives that and nothing else. The types in each
// group are declared once in New().
#define DEMIR_GROUP_CABLES (1<<0)
#define DEMIR_GROUP_PIPES (1<<1)
#define DEMIR_GROUP_DISPOSALS (1<<2)
#define DEMIR_GROUP_UNDERFLOOR (DEMIR_GROUP_CABLES | DEMIR_GROUP_PIPES | DEMIR_GROUP_DISPOSALS)

// Layer 3 is the unsuffixed mapping family. The other layers have separate visible and hidden
// mapping subtypes, so registering those more-derived paths keeps them out of the layer 3 group.
#define DEMIR_NODE_BLOCKERS list(/turf/closed, /obj/effect/spawner/structure/window)
#define DEMIR_NODE_PIPE_FAMILY(Fulltype) \
	demir_node_group(Fulltype, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/visible/layer1, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/hidden/layer1, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/visible/layer2, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/hidden/layer2, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/visible/layer4, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/hidden/layer4, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/visible/layer5, DEMIR_NODE_BLOCKERS); \
	demir_node_group(Fulltype/hidden/layer5, DEMIR_NODE_BLOCKERS)

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

// tgstation documents a null canSmoothWith on a smoothing atom as "smooth with the same type".
// Its bitmask helper only compares explicit groups, though, and snow mineral walls rely on the
// null form. Give those mineral walls a private type-keyed group for the preview bake.
/turf/closed/wall/mineral/demir_prepare_smoothing()
	. = ..()
	if(canSmoothWith || !(smoothing_flags & USES_SMOOTHING))
		return
	var/list/self_smoothing_group = list()
	self_smoothing_group[type] = TRUE
	canSmoothWith = self_smoothing_group
	smoothing_groups = smoothing_groups ? smoothing_groups.Copy() : list()
	smoothing_groups[type] = TRUE

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
	else if(plane == LIGHTING_PLANE)
		if(blend_mode == BLEND_ADD)
			demir_overlay_light = 1
		else if(blend_mode == BLEND_SUBTRACT)
			demir_overlay_light = -1
		else
			alpha = 0
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

// Cables and pipes are covered by a floor tile through /datum/element/undertile, which reacts to
// COMSIG_OBJ_HIDE from levelupdate(). No signals or elements run here, so read the turf's own
// accessibility instead and apply the same result.
// Null for anything the element never covers, so a shown one is told apart from an untouched one.
/atom/movable/proc/demir_underfloor_shown()
	return null

/obj/structure/cable/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return profile.show_cables

/obj/machinery/power/terminal/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return profile.show_cables

// setup_hiding() runs only when hide is set, so the /visible subtypes never take the element.
/obj/machinery/atmospherics/pipe/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return hide ? profile.show_pipes : null

/obj/machinery/duct/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return profile.show_pipes

/obj/structure/disposalpipe/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return profile.show_disposals

/obj/structure/disposalconstruct/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return profile.show_disposals

/atom/movable/proc/demir_apply_underfloor()
	var/shown = demir_underfloor_shown()
	if(isnull(shown))
		return

	var/turf/our_turf = loc
	if(!isturf(our_turf) || our_turf.underfloor_accessibility >= UNDERFLOOR_VISIBLE)
		return

	if(!shown)
		alpha = 0
		return

	var/datum/demir/tgstation/profile = demir_profile()
	if(profile.fade_underfloor)
		// undertile.dm undefines its own ALPHA_UNDERTILE before this profile is included.
		alpha = 128

	// The element sinks whatever it leaves visible onto the floor plane, and only what is not already
	// there. Without it a pipe covers the machines standing on its own tile; with the guard a cable
	// keeps the per-cable-layer offsets it draws its ordering from. Assigned rather than
	// SET_PLANE_IMPLICIT because the multi-z plane offsets that consults mean nothing to a bake.
	if(plane != FLOOR_PLANE)
		plane = FLOOR_PLANE
		layer = BELOW_CATWALK_LAYER

/obj/machinery/door/airlock/proc/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	update_appearance(UPDATE_ICON)

// A docking port stands on one tile but claims a rectangle around it, rotated by its dir. This is
// return_coords() reduced to a south-west offset and an extent.
/obj/docking_port/proc/demir_region_extent()
	return list(width, height, dwidth, dheight)

// Mobile ports are sized by calculate_docking_port_information() when their template loads, so a
// mapped one still reads zero. The template is the map being edited, so its bounds and the port's
// own tile stand in for the template size and port_x_offset/port_y_offset.
/obj/docking_port/mobile/demir_region_extent()
	if(width && height)
		return ..()

	var/template_width = world.maxx
	var/template_height = world.maxy
	var/port_x_offset = x
	var/port_y_offset = y

	var/rotated_width = template_width
	var/rotated_height = template_height
	if(dir == EAST || dir == WEST)
		rotated_width = template_height
		rotated_height = template_width

	// The native switch reads the pre-swap template size, not the rotated one.
	var/offset_width = port_x_offset - 1
	var/offset_height = port_y_offset - 1
	switch(dir)
		if(EAST)
			offset_width = template_height - port_y_offset
			offset_height = port_x_offset - 1
		if(SOUTH)
			offset_width = template_width - port_x_offset
			offset_height = template_height - port_y_offset
		if(WEST)
			offset_width = port_y_offset - 1
			offset_height = template_width - port_x_offset

	return list(rotated_width, rotated_height, offset_width, offset_height)

/obj/docking_port/proc/demir_highlight()
	var/list/extent = demir_region_extent()
	var/port_width = extent[1]
	var/port_height = extent[2]
	var/port_dwidth = extent[3]
	var/port_dheight = extent[4]
	if(port_width < 1 || port_height < 1)
		return null

	var/cos = 1
	var/sin = 0
	switch(dir)
		if(WEST)
			cos = 0
			sin = 1
		if(SOUTH)
			cos = -1
			sin = 0
		if(EAST)
			cos = 0
			sin = -1

	// return_coords() yields two opposite corners, not a min and a max.
	var/near_x = (-port_dwidth * cos) - (-port_dheight * sin)
	var/near_y = (-port_dwidth * sin) + (-port_dheight * cos)
	var/far_x = ((-port_dwidth + port_width - 1) * cos) - ((-port_dheight + port_height - 1) * sin)
	var/far_y = ((-port_dwidth + port_width - 1) * sin) + ((-port_dheight + port_height - 1) * cos)

	return list(
		"x" = min(near_x, far_x),
		"y" = min(near_y, far_y),
		"width" = abs(far_x - near_x) + 1,
		"height" = abs(far_y - near_y) + 1,
		"color" = "#ff8000",
		"when" = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED,
		"label" = name || "docking port",
	)

/datum/demir/tgstation/highlights(atom/target)
	if(!istype(target, /obj/docking_port))
		return null

	var/obj/docking_port/port = target
	var/list/highlight = port.demir_highlight()

	return highlight ? list(highlight) : null

/datum/demir/tgstation/proc/register_disposal_node_orientations()
	for(var/type_path in typesof(/obj/structure/disposalpipe))
		var/obj/structure/disposalpipe/pipe_type = type_path
		var/extra = initial(pipe_type.initialize_dirs)
		for(var/direction in GLOB.cardinals)
			var/openings = 0
			if(extra != DISP_DIR_NONE)
				openings = direction
				if(extra & DISP_DIR_LEFT)
					openings |= turn(direction, 90)
				if(extra & DISP_DIR_RIGHT)
					openings |= turn(direction, -90)
				if(extra & DISP_DIR_FLIP)
					openings |= REVERSE_DIR(direction)
			demir_node_orientation(type_path, direction, openings)
		for(var/direction in GLOB.diagonals)
			demir_node_orientation(type_path, direction, extra == DISP_DIR_NONE ? 0 : direction)

/datum/demir/tgstation
	default = TRUE
	var/smooth = TRUE
	var/lighting = TRUE
	var/show_cables = TRUE
	var/show_pipes = TRUE
	var/show_disposals = TRUE
	var/fade_underfloor = FALSE

	New()
		..()
		if(!GLOB)
			GLOB = new /datum/controller/global_vars/demir_preview

		// A group is a type and everything under it, so the /visible atmos pipes join
		// DEMIR_GROUP_PIPES too. Re-deriving one that was never hidden costs a preview and
		// changes nothing.
		demir_define_group(DEMIR_GROUP_CABLES, /obj/structure/cable)
		demir_define_group(DEMIR_GROUP_CABLES, /obj/machinery/power/terminal)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/atmospherics/pipe)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/duct)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalpipe)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalconstruct)

		// The node tool copies the seeded map prefab along a cardinal route and keeps
		// every route out of closed turfs. Pipe color/network families and piping layers
		// are separate groups so adjacent supply, scrubber and stacked pipes never merge.
		demir_node_group(/obj/structure/cable, /turf/closed)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/yellow)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/general)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/cyan)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/green)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/orange)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/purple)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/dark)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/brown)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/violet)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/pink)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/scrubbers)
		DEMIR_NODE_PIPE_FAMILY(/obj/machinery/atmospherics/pipe/smart/simple/supply)
		demir_node_group(/obj/machinery/atmospherics/pipe/smart/manifold4w/supply, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/machinery/atmospherics/pipe/smart/manifold4w/scrubbers, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/machinery/duct, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/structure/disposalpipe, DEMIR_NODE_BLOCKERS, /obj/structure/disposalpipe/segment)
		demir_node_group(/obj/structure/disposalconstruct, DEMIR_NODE_BLOCKERS)
		register_disposal_node_orientations()

// ui() rolls its writes back on every frame but the one the viewer touched something on, so the
// profile's own vars are where panel state belongs. Every other hook reads them off src, and the
// frame that changes one re-derives appearances, highlights and lighting.
/datum/demir/tgstation/ui(atom/target)
	if(!imgui_begin("Demir"))
		imgui_end()
		return

	imgui_separator("Baking")
	var/smoothed = imgui_checkbox("Smooth walls", src.smooth)
	if(smoothed != src.smooth)
		src.smooth = smoothed
		// Smoothing is declared at a hundred-odd scattered types, so there is no honest group for it.
		demir_rebake(DEMIR_BAKE_APPEARANCE)

	var/lit = imgui_checkbox("Lighting", src.lighting)
	if(lit != src.lighting)
		src.lighting = lit
		// Every area carries the fullbright flag, so this one is not worth narrowing.
		demir_rebake(DEMIR_BAKE_LIGHT)

	imgui_separator("Under-floor")
	var/cables = imgui_checkbox("Cables", src.show_cables)
	if(cables != src.show_cables)
		src.show_cables = cables
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_CABLES)

	var/pipes = imgui_checkbox("Pipes", src.show_pipes)
	if(pipes != src.show_pipes)
		src.show_pipes = pipes
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_PIPES)

	var/disposals = imgui_checkbox("Disposals", src.show_disposals)
	if(disposals != src.show_disposals)
		src.show_disposals = disposals
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_DISPOSALS)

	var/fade = imgui_checkbox("Semitransparent", src.fade_underfloor)
	if(fade != src.fade_underfloor)
		src.fade_underfloor = fade
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_UNDERFLOOR)

	imgui_end()

/datum/demir/tgstation/prepare(atom/target)
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
	if(istype(target, /obj/structure/closet))
		var/obj/structure/closet/closet = target
		closet.PopulateContents()

/datum/demir/tgstation/bake(atom/target)
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
	else if(src.smooth && (target.smoothing_flags & USES_SMOOTHING))
		target.smooth_icon()
	if(ismovable(target))
		var/atom/movable/movable_target = target
		movable_target.demir_add_overlay_light()
		movable_target.demir_apply_underfloor()
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
	var/list/light_offset = get_light_offset()
	// Native lighting adds these offsets to its sampling coordinates, which moves the source
	// origin in the opposite direction.
	demir_light_offset_x = -light_offset[1]
	demir_light_offset_y = -light_offset[2]
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
	// Switching lighting off lights the whole map rather than blacking it out, which is what a
	// mapper wants from the toggle.
	var/datum/demir/tgstation/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/tgstation/light(atom/target)
	target.demir_apply_light()

// Connection channels are opaque to rmd. Keep the controlled type and DM value kind in the key
// so an airlock id_tag cannot collide with a poddoor id, or a numeric ID with the same text.
/proc/get_connection_key(kind, value)
	if(isnull(value))
		return null
	if(isnum(value))
		return "[kind]:number:[value]"
	if(istext(value))
		return "[kind]:text:[value]"
	return null

/datum/demir/tgstation/connections(atom/target)
	var/channel
	var/roles
	if(istype(target, /obj/machinery/button/door))
		var/obj/machinery/button/door/button = target
		var/controller_id = button.id
		if(!controller_id)
			// These are the initial IDs of the controller assemblies created by setup_device().
			controller_id = button.normaldoorcontrol ? "badmin" : -1
		channel = get_connection_key(button.normaldoorcontrol ? "airlock" : "poddoor", controller_id)
		roles = DEMIR_CONNECTION_SOURCE
	else if(istype(target, /obj/machinery/door/poddoor))
		var/obj/machinery/door/poddoor/poddoor = target
		channel = get_connection_key("poddoor", poddoor.id)
		roles = DEMIR_CONNECTION_TARGET
	else if(istype(target, /obj/machinery/door/airlock))
		var/obj/machinery/door/airlock/airlock = target
		channel = get_connection_key("airlock", airlock.id_tag)
		roles = DEMIR_CONNECTION_TARGET

	var/list/connections = list()
	if(channel)
		connections[channel] = roles
	return connections

// A border object only blocks the side it stands on, and the lighting corners care about
// whether the whole tile is concealed. IS_OPAQUE_TURF wants ALL_CARDINALS.
/atom/movable/demir_apply_light()
	if(opacity && (flags_1 & ON_BORDER_1))
		demir_blocks_light = 0
	return ..()

#undef DEMIR_NODE_PIPE_FAMILY
#undef DEMIR_GROUP_CABLES
#undef DEMIR_GROUP_PIPES
#undef DEMIR_GROUP_DISPOSALS
#undef DEMIR_GROUP_UNDERFLOOR

#endif
