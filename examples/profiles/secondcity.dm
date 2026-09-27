// rmd embeds this integration for the Forced bundled profile setting. To make the profile part of
// a SecondCity checkout instead, copy it there and `#include` it from `tgstation.dme`, the guard
// keeps BYOND compiling it to nothing. The build tool passes CBT, which MAP_SWITCH needs to pick the
// runtime rock and wall icons, so also `#define CBT` inside the dme's `__DEMIR_BAKE__` branch.
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
#define DEMIR_GROUP_FRILLS (1<<3)

#define DEMIR_NODE_BLOCKERS list(/turf/closed, /obj/effect/spawner/structure/window)

// Run the codebase's smoothing setup without starting game subsystems.
/datum/controller/subsystem/mapping/demir_preview/New()
	SSmapping = src

/datum/controller/subsystem/mapping/demir_preview/level_trait(z, trait)
	return null

/datum/controller/subsystem/overlays/demir_preview/New()
	SSoverlays = src
	stats = list()

/datum/controller/subsystem/atoms/demir_preview/New()
	SSatoms = src
	initialized = INITIALIZATION_INSSATOMS

// INITIALIZE_IMMEDIATE types, organs among them, initialize from New() whatever SSatoms says, so a
// freezer's PopulateContents() would run the whole item Initialize(). Leave them like placed atoms.
/datum/controller/subsystem/atoms/demir_preview/InitAtom(atom/A, from_template = FALSE, list/arguments)
	return FALSE

/datum/controller/subsystem/machines/demir_preview/New()
	SSmachines = src

// Overlay lights register with grid cells so mobs can count the light they stand in.
/datum/controller/subsystem/spatial_grid/demir_preview/New()
	SSspatial_grid = src

/datum/controller/subsystem/spatial_grid/demir_preview/get_cells_in_range(atom/center, range)
	return list()

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
	if(!SSmachines)
		SSmachines = new /datum/controller/subsystem/machines/demir_preview
	if(!SSspatial_grid)
		SSspatial_grid = new /datum/controller/subsystem/spatial_grid/demir_preview
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
	InitGlobalstarlight_range()
	InitGlobalstarlight_power()
	InitGlobalstarlight_color()
	InitGlobalareas()
	InitGlobalareas_by_type()
	InitGlobalstation_levels_cache()

/atom/proc/demir_prepare_smoothing()
	SETUP_SMOOTHING()
	if(uses_integrity)
		atom_integrity = max_integrity

// Grass and ash is problematic, we need to handle this like this, gg.
/turf/open/misc/Initialize(mapload)
	return INITIALIZE_HINT_NORMAL

// The light_system switch that closes /atom/movable/Initialize(). The component draws its own
// mask and cone.
/atom/movable/proc/demir_add_overlay_light()
	switch(light_system)
		if(OVERLAY_LIGHT)
			AddComponent(/datum/component/overlay_lighting)
		if(OVERLAY_LIGHT_DIRECTIONAL)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE)
		if(OVERLAY_LIGHT_BEAM)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE, is_beam = TRUE)

// The lighting subsystem's /datum/light_source is what the light hook stands in for, so the
// codebase's own set_light*() calls only leave their values on the atom.
/atom/update_light()
	return

/image/proc/demir_tag_emissive()
	if(plane == EMISSIVE_PLANE && islist(color))
		var/list/color_matrix = color
		if(color_matrix[17] || color_matrix[18])
			demir_emissive = TRUE
		else if(color_matrix[16] && !color_matrix[19])
			demir_emissive_blocker = TRUE
	else if(plane == O_LIGHTING_VISUAL_PLANE)
		if(blend_mode == BLEND_ADD)
			demir_overlay_light = 1
		else if(blend_mode == BLEND_SUBTRACT)
			demir_overlay_light = -1
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

// The duct's post_machine_initialize() links both ends and repaints the neighbor, which a
// per-placement bake would export as half-linked neighbor states, so this mirrors its checks.
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

// A low wall builds its window in Initialize(). Preview the windows the low walls around it would
// build, so the glass smooths into its neighbors, and keep only this wall's.
/obj/structure/platform/lowwall/proc/demir_bake_window()
	var/list/nearby_turfs = list(loc)
	for(var/direction in GLOB.alldirs)
		nearby_turfs += get_step(src, direction)

	var/list/all_spawned = list()
	var/obj/structure/window/owned_window
	for(var/turf/nearby_turf in nearby_turfs)
		for(var/obj/structure/platform/lowwall/low_wall in nearby_turf)
			var/window_type = low_wall.window
			if(!window_type)
				continue

			var/obj/structure/window/spawned = new window_type(nearby_turf)
			spawned.demir_prepare_smoothing()
			all_spawned += spawned
			if(low_wall == src)
				owned_window = spawned

	for(var/obj/structure/window/spawned in all_spawned)
		if(spawned.smoothing_flags & USES_SMOOTHING)
			spawned.smooth_icon()

	if(!owned_window)
		return

	var/image/preview = new
	preview.appearance = owned_window.appearance
	// Appending to an obj's overlays in place exports nothing, so assign a new list.
	var/list/new_overlays = list()
	for(var/existing in overlays)
		new_overlays += existing
	new_overlays += preview
	overlays = new_overlays

// The frill's Initialize() adds the seethrough component this reads, and a bake never runs it.
/obj/effect/wall_frill/update_seethrough()
	return

// set_smoothed_icon_state() stands a frill on the turf to the north. A bake only exports the
// placement it runs on, so the wall draws its frill one tile up, on the frill's own plane and layer.
/turf/closed/wall/vampwall/proc/demir_bake_frill()
	var/datum/demir/secondcity/profile = demir_profile()
	if(!wall_frill || !profile.show_frills)
		return

	var/image/preview = new
	preview.appearance = wall_frill.appearance
	preview.pixel_y += ICON_SIZE_Y
	overlays += preview

// Cables and pipes are covered by a floor tile through /datum/element/undertile, which reacts to
// COMSIG_OBJ_HIDE from levelupdate(). No signals or elements run here, so read the turf's own
// accessibility instead and apply the same result.
// Null for anything the element never covers, so a shown one is told apart from an untouched one.
/atom/movable/proc/demir_underfloor_shown()
	return null

/obj/structure/cable/demir_underfloor_shown()
	var/datum/demir/secondcity/profile = demir_profile()
	return profile.show_cables

/obj/machinery/power/terminal/demir_underfloor_shown()
	var/datum/demir/secondcity/profile = demir_profile()
	return profile.show_cables

// setup_hiding() runs only when hide is set, so the /visible subtypes never take the element.
/obj/machinery/atmospherics/pipe/demir_underfloor_shown()
	var/datum/demir/secondcity/profile = demir_profile()
	return hide ? profile.show_pipes : null

/obj/machinery/duct/demir_underfloor_shown()
	var/datum/demir/secondcity/profile = demir_profile()
	return profile.show_pipes

/obj/structure/disposalpipe/demir_underfloor_shown()
	var/datum/demir/secondcity/profile = demir_profile()
	return profile.show_disposals

/obj/structure/disposalconstruct/demir_underfloor_shown()
	var/datum/demir/secondcity/profile = demir_profile()
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

	var/datum/demir/secondcity/profile = demir_profile()
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

// Inline in the airlock's Initialize().
/obj/machinery/door/airlock/proc/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	update_appearance(UPDATE_ICON)

// A mapped mobile port is sized when its shuttle template loads. The template is the map being
// edited, so its bounds stand in for the file preload_size() would parse.
/datum/map_template/shuttle/demir_preview/preload_size(path, cache)
	width = world.maxx
	height = world.maxy

/obj/docking_port/proc/demir_highlight()
	if(istype(src, /obj/docking_port/mobile) && !(width && height))
		var/obj/docking_port/mobile/port = src
		var/datum/map_template/shuttle/demir_preview/template = new
		template.port_x_offset = x
		template.port_y_offset = y
		port.calculate_docking_port_information(template)
	if(width < 1 || height < 1)
		return null

	// Two opposite corners, not a min and a max.
	var/list/corners = return_coords(0, 0)
	return list(
		"x" = min(corners[1], corners[3]),
		"y" = min(corners[2], corners[4]),
		"width" = abs(corners[3] - corners[1]) + 1,
		"height" = abs(corners[4] - corners[2]) + 1,
		"color" = "#ff8000",
		"when" = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED,
		"label" = name || "docking port",
	)

/datum/demir/secondcity/highlights(atom/target)
	if(!istype(target, /obj/docking_port))
		return null

	var/obj/docking_port/port = target
	var/list/highlight = port.demir_highlight()

	return highlight ? list(highlight) : null

// The DISP_DIR_* rule is inline in the pipe's Initialize().
/datum/demir/secondcity/proc/register_disposal_node_orientations()
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

/datum/demir/secondcity
	default = TRUE
	var/smooth = TRUE
	var/show_frills = TRUE
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
		demir_define_group(DEMIR_GROUP_FRILLS, /turf/closed/wall/vampwall)

		// The node tool copies the seeded map prefab along a cardinal route and keeps
		// every route out of closed turfs. Each pipe color and piping layer the mapping
		// helpers declare is its own group, so adjacent supply, scrubber and stacked pipes
		// never merge.
		demir_node_group(/obj/structure/cable, /turf/closed)
		for(var/pipe_path in typesof(/obj/machinery/atmospherics/pipe/smart/simple))
			var/obj/machinery/atmospherics/pipe/pipe = pipe_path
			var/obj/machinery/atmospherics/pipe/parent = initial(pipe.parent_type)
			if(initial(pipe.color) != initial(parent.color) || initial(pipe.piping_layer) != initial(parent.piping_layer))
				demir_node_group(pipe_path, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/machinery/atmospherics/pipe/smart/manifold4w/supply, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/machinery/atmospherics/pipe/smart/manifold4w/scrubbers, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/machinery/duct, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/structure/disposalpipe, DEMIR_NODE_BLOCKERS, /obj/structure/disposalpipe/segment)
		demir_node_group(/obj/structure/disposalconstruct, DEMIR_NODE_BLOCKERS)
		register_disposal_node_orientations()

// ui() rolls its writes back on every frame but the one the viewer touched something on, so the
// profile's own vars are where panel state belongs. Every other hook reads them off src, and the
// frame that changes one re-derives appearances, highlights and lighting.
/datum/demir/secondcity/ui(atom/target)
	if(!imgui_begin("Demir"))
		imgui_end()
		return

	imgui_separator("Baking")
	var/smoothed = imgui_checkbox("Smooth walls", src.smooth)
	if(smoothed != src.smooth)
		src.smooth = smoothed
		// Smoothing is declared at a hundred-odd scattered types, so there is no honest group for it.
		demir_rebake(DEMIR_BAKE_APPEARANCE)

	// A frill covers the whole tile north of its wall.
	var/frills = imgui_checkbox("Wall frills", src.show_frills)
	if(frills != src.show_frills)
		src.show_frills = frills
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_FRILLS)

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

/datum/demir/secondcity/prepare(atom/target)
	// The bake builds atoms without New(), which is where an area sets up its power bookkeeping.
	if(isarea(target))
		target.New()
	target.demir_prepare_smoothing()
	// Inline in the flashlight's Initialize().
	if(istype(target, /obj/item/flashlight))
		var/obj/item/flashlight/flashlight = target
		if(flashlight.start_on)
			flashlight.set_light_on(TRUE)
	if(istype(target, /turf/open/floor/light))
		var/turf/open/floor/light/light_floor = target
		light_floor.update_appearance()
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

/datum/demir/secondcity/bake(atom/target)
	if(istype(target, /obj/effect/spawner/structure/window))
		var/obj/effect/spawner/structure/window/spawner = target
		spawner.demir_bake_spawned_appearance()
	else if(istype(target, /obj/structure/cable) && !istype(target, /obj/structure/cable/multilayer))
		var/obj/structure/cable/cable = target
		cable.connect_cable(TRUE)
		cable.LateInitialize()
	else if(istype(target, /obj/machinery/duct))
		var/obj/machinery/duct/duct = target
		duct.demir_bake_connections()
	else if(istype(target, /obj/machinery/door/airlock))
		var/obj/machinery/door/airlock/airlock = target
		airlock.demir_bake_appearance()
	else if(istype(target, /obj/machinery/atmospherics))
		var/obj/machinery/atmospherics/atmos_target = target
		atmos_target.demir_bake_connections()
	else if(istype(target, /obj/structure/platform/lowwall))
		var/obj/structure/platform/lowwall/low_wall = target
		if(src.smooth)
			low_wall.smooth_icon()

		if(low_wall.window)
			low_wall.demir_bake_window()
	else if(src.smooth && (target.smoothing_flags & USES_SMOOTHING))
		if(istype(target, /turf/open/misc/grass) || istype(target, /turf/open/misc/ashplanet))
			target.Initialize(TRUE)

		target.smooth_icon()
		if(istype(target, /turf/closed/wall/vampwall))
			var/turf/closed/wall/vampwall/wall = target
			wall.demir_bake_frill()
	if(ismovable(target))
		var/atom/movable/movable_target = target
		movable_target.demir_add_overlay_light()
		movable_target.demir_apply_underfloor()
	target.demir_tag_emissive()

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

// Fixtures ship with `on = FALSE`. Initialize() aims the light through setDir() and
// post_machine_initialize() switches it on from the area's lightswitch and power.
/obj/machinery/light/demir_apply_light()
	setDir(dir)
	power_change()
	return ..()

// Space is a fullbright area, so its tiles never hold a lighting object and only the ones
// touching a statically lit turf call enable_starlight().
/turf/open/space
	demir_light_edge_only = 1

/turf/open/space/demir_apply_light()
	demir_light_range = GLOB.starlight_range
	demir_light_power = GLOB.starlight_power
	demir_light_color = GLOB.starlight_color

// The city has no space. moonlight.dm instead lights every floor, misc and water turf of an
// outdoors area that has no light of its own, through an element added in Initialize().
/turf/open/proc/demir_moonlit()
	if(!istype(src, /turf/open/floor) && !istype(src, /turf/open/misc) && !istype(src, /turf/open/water))
		return FALSE

	if(light_power && light_range)
		return FALSE

	var/area/our_area = loc

	return our_area.outdoors

/turf/open/demir_apply_light()
	if(!demir_moonlit())
		return ..()

	demir_light_range = max(GLOB.starlight_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_power = GLOB.starlight_power
	demir_light_color = GLOB.starlight_color
	demir_light_height = light_height

/area/demir_apply_light()
	// Switching lighting off lights the whole map rather than blacking it out, which is what a
	// mapper wants from the toggle.
	var/datum/demir/secondcity/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/secondcity/light(atom/target)
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

/datum/demir/secondcity/connections(atom/target)
	var/channel
	var/roles
	if(istype(target, /obj/machinery/button/door))
		var/obj/machinery/button/door/button = target
		if(!button.device)
			button.setup_device(TRUE)
		var/obj/item/assembly/control/controller = button.device
		var/kind = istype(controller, /obj/item/assembly/control/airlock) ? "airlock" : "poddoor"
		channel = get_connection_key(kind, controller?.id)
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

#undef DEMIR_NODE_BLOCKERS
#undef DEMIR_GROUP_CABLES
#undef DEMIR_GROUP_PIPES
#undef DEMIR_GROUP_DISPOSALS
#undef DEMIR_GROUP_UNDERFLOOR
#undef DEMIR_GROUP_FRILLS

#endif
