// rmd embeds this profile for the Forced bundled profile setting. To make the profile part of a
// Monkestation checkout instead, copy it there and `#include` it from `tgstation.dme` after
// `// END_INCLUDE`. BYOND skips it because of the guard below. The build tool passes CBT, which
// MAP_SWITCH needs to pick the runtime rock and wall icons, so also `#define CBT` inside the dme's
// `__DEMIR_BAKE__` branch.
//
// The profile runs the codebase's own smoothing and icon code without starting any subsystem.
// `Initialize(TRUE)` faults on nearly every atom, so each hook sets up only the vars its icon code
// reads.
#ifdef __DEMIR_BAKE__

#define DEMIR_GROUP_CABLES (1<<0)
#define DEMIR_GROUP_PIPES (1<<1)
#define DEMIR_GROUP_DISPOSALS (1<<2)
#define DEMIR_GROUP_UNDERFLOOR (DEMIR_GROUP_CABLES | DEMIR_GROUP_PIPES | DEMIR_GROUP_DISPOSALS)
#define DEMIR_GROUP_POWER (1<<3)

#define DEMIR_NODE_BLOCKERS list(/turf/closed, /obj/effect/spawner/structure/window)

// Stand-ins for the subsystems the smoothing and icon code reads.
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

/datum/controller/subsystem/machines/demir_preview/New()
	SSmachines = src

/datum/controller/subsystem/icon_smooth/demir_preview/New()
	SSicon_smooth = src

// The bake smooths every atom directly.
/datum/controller/subsystem/icon_smooth/demir_preview/add_to_queue(atom/thing)
	return

// Initialize() builds the department palette and recolors the windows in each area. The bake
// colors one window at a time, so this stores the palette by area type instead.
/datum/controller/subsystem/station_coloring/demir_preview
	var/list/area_colors

/datum/controller/subsystem/station_coloring/demir_preview/New()
	SSstation_coloring = src
	area_colors = list()
	Initialize()

/datum/controller/subsystem/station_coloring/demir_preview/color_area_objects(list/possible_areas, color)
	for(var/type in possible_areas)
		area_colors[type] = color

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
	if(!SSicon_smooth)
		SSicon_smooth = new /datum/controller/subsystem/icon_smooth/demir_preview
	if(!SSstation_coloring)
		SSstation_coloring = new /datum/controller/subsystem/station_coloring/demir_preview
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
	InitGlobalstarlight_color()
	InitGlobalstation_levels_cache()

/atom/proc/demir_prepare_smoothing()
	SETUP_SMOOTHING()
	if(uses_integrity)
		atom_integrity = max_integrity

// Grass and ash is problematic, we need to handle this like this, gg.
/turf/open/misc/Initialize(mapload)
	return INITIALIZE_HINT_NORMAL

// Same light_system switch as the end of /atom/movable/Initialize().
/atom/movable/proc/demir_add_overlay_light()
	switch(light_system)
		if(OVERLAY_LIGHT)
			AddComponent(/datum/component/overlay_lighting)
		if(OVERLAY_LIGHT_DIRECTIONAL)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE)
		if(MOVABLE_LIGHT_BEAM)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE, is_beam = TRUE)

// This tracks the dynamic_lumcount mobs read, through view() of the live map.
/datum/component/overlay_lighting/get_new_turfs()
	return

// The light hook replaces /datum/light_source, so set_light() only stores its values on the
// atom.
/atom/update_light()
	return

// PARSE_LIGHT_COLOR takes a light source, and its New() registers with the lighting subsystem.
/datum/light_source/demir_preview/New()
	return

/image/proc/demir_tag_emissive()
	if(plane == EMISSIVE_PLANE && islist(color))
		var/list/color_matrix = color
		if(color_matrix[17] || color_matrix[18])
			demir_emissive = TRUE
		else if(color_matrix[16] && !color_matrix[19])
			demir_emissive_blocker = TRUE
	else if(plane == O_LIGHTING_VISUAL_PLANE)
		// The plane master treats everything on this plane as light, whatever its blend mode.
		demir_overlay_light = 1
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

// atmos_init() finds the nodes and redraws the device. It does not build pipe networks.
/obj/machinery/atmospherics/demir_bake_appearance()
	// GAGS icons are built at runtime, so keep the mapped icon_state.
	var/preview_state = icon_state
	var/gags = greyscale_config
	greyscale_config = null
	nodes = list()
	nodes.len = device_type
	atmos_init()
	if(gags)
		icon_state = preview_state

/obj/machinery/atmospherics/pipe/layer_manifold/demir_bake_appearance()
	icon_state = "manifoldlayer_center"
	return ..()

/obj/machinery/atmospherics/pipe/multiz/demir_bake_appearance()
	icon_state = ""
	center = mutable_appearance(icon, "adapter_center", layer = HIGH_OBJ_LAYER)
	pipe = mutable_appearance(icon, "pipe-[piping_layer]")
	return ..()

// connect_duct() builds a ductnet while it links, so this copies its checks and keeps only the
// connection bits the icon state reads.
/obj/machinery/duct/demir_bake_appearance()
	if(!active)
		return
	var/new_connects = (dumb || lock_connects) ? connects : NONE
	for(var/check_dir in GLOB.cardinals)
		if(dumb && !(check_dir & connects))
			continue
		var/turf/step_turf = get_step(src, check_dir)
		for(var/obj/machinery/duct/other_duct in step_turf)
			if(!other_duct.active)
				continue
			if(!dumb && other_duct.dumb && !(REVERSE_DIR(check_dir) & other_duct.connects))
				continue
			if(dumb && other_duct.dumb && !(connects & other_duct.connects))
				continue
			if(!(duct_layer & other_duct.duct_layer))
				continue
			if(duct_color != other_duct.duct_color && !(ignore_colors || other_duct.ignore_colors))
				continue
			new_connects |= check_dir
			break
	if(!lock_connects)
		connects = new_connects
	update_icon_state()

// Initialize() takes the default glass color, then SSstation_coloring recolors it by department.
/obj/structure/window/proc/demir_bake_color()
	if(!uses_color)
		return
	var/datum/controller/subsystem/station_coloring/demir_preview/coloring = SSstation_coloring
	var/area/window_area = get_area(src)
	change_color(coloring.area_colors[window_area?.type] || coloring.get_default_color())

// Same check as the window's Initialize(), which adds a sill under every colored fulltile window.
/turf/proc/demir_gets_window_sill()
	for(var/obj/structure/window/window in src)
		if(window.uses_color && window.fulltile)
			return TRUE
	for(var/obj/effect/spawner/structure/window/spawner in src)
		for(var/spawn_type in spawner.spawn_list)
			if(!ispath(spawn_type, /obj/structure/window))
				continue
			var/obj/structure/window/window_type = spawn_type
			if(initial(window_type.uses_color) && initial(window_type.fulltile))
				return TRUE
	return FALSE

// Sills smooth with their neighbors, so build those too and draw only the one on this turf. In
// game the sill is a separate object, so it does not take the window's color.
/atom/movable/proc/demir_preview_window_sill()
	var/list/nearby_turfs = list(loc)
	for(var/direction in list(NORTH, SOUTH, EAST, WEST, NORTHEAST, NORTHWEST, SOUTHEAST, SOUTHWEST))
		nearby_turfs += get_step(src, direction)

	var/list/sills = list()
	var/obj/structure/window_sill/own_sill
	for(var/turf/nearby_turf in nearby_turfs)
		if(!nearby_turf.demir_gets_window_sill())
			continue
		var/obj/structure/window_sill/sill = new(nearby_turf)
		sill.demir_prepare_smoothing()
		sills += sill
		if(nearby_turf == loc)
			own_sill = sill

	for(var/obj/structure/window_sill/sill in sills)
		sill.smooth_icon()

	if(!own_sill)
		return

	var/image/preview = new
	preview.appearance = own_sill.appearance
	preview.appearance_flags |= RESET_COLOR | RESET_ALPHA
	overlays += preview

// Window spawners create their grille and windows in Initialize(). Run it for this spawner and
// its neighbors so the windows smooth together, then draw only this spawner's atoms.
/obj/effect/spawner/structure/window/demir_bake_appearance()
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
				if(istype(spawned, /obj/structure/window))
					var/obj/structure/window/window = spawned
					window.demir_bake_color()
				all_spawned += spawned
				if(spawner == src)
					owned_spawned += spawned

	for(var/atom/movable/spawned in all_spawned)
		if(spawned.smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK))
			spawned.smooth_icon()

	icon = null
	icon_state = null
	overlays = list()
	for(var/atom/movable/spawned in owned_spawned)
		var/image/preview = new
		preview.appearance = spawned.appearance
		overlays += preview

	demir_preview_window_sill()

// /datum/element/undertile hides cables and pipes on COMSIG_OBJ_HIDE. Signals do not run in a
// bake, so this reads the turf's underfloor_accessibility instead. Null means the element never
// applies to the type.
/atom/movable/proc/demir_underfloor_shown()
	return null

/obj/structure/cable/demir_underfloor_shown()
	var/datum/demir/monkestation/profile = demir_profile()
	return profile.show_cables

/obj/machinery/power/terminal/demir_underfloor_shown()
	var/datum/demir/monkestation/profile = demir_profile()
	return profile.show_cables

// setup_hiding() runs only when hide is set, so the /visible subtypes never take the element.
/obj/machinery/atmospherics/pipe/demir_underfloor_shown()
	var/datum/demir/monkestation/profile = demir_profile()
	return hide ? profile.show_pipes : null

/obj/machinery/duct/demir_underfloor_shown()
	var/datum/demir/monkestation/profile = demir_profile()
	return profile.show_pipes

/obj/structure/disposalpipe/demir_underfloor_shown()
	var/datum/demir/monkestation/profile = demir_profile()
	return profile.show_disposals

/obj/structure/disposalconstruct/demir_underfloor_shown()
	var/datum/demir/monkestation/profile = demir_profile()
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

	var/datum/demir/monkestation/profile = demir_profile()
	if(profile.fade_underfloor)
		// ALPHA_UNDERTILE is undefined at the end of undertile.dm.
		alpha = 128

	// The element moves what it leaves visible to FLOOR_PLANE, so pipes draw under the machines on
	// their tile. Cables already there keep the layer offsets they sort by. SET_PLANE_IMPLICIT is
	// skipped because the bake has no multi-z plane offsets. A pipe inside a wall keeps its plane,
	// or the wall would cover it.
	if(plane != FLOOR_PLANE && !isclosedturf(our_turf))
		plane = FLOOR_PLANE

// Same as the airlock's Initialize().
/obj/machinery/door/airlock/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	update_appearance(UPDATE_ICON)

// A mobile port gets its size when its shuttle template loads. Here the template is the map being
// edited, so use its bounds instead of parsing a file.
/datum/map_template/shuttle/demir_preview/preload_size(path, cache)
	width = world.maxx
	height = world.maxy

/obj/docking_port/proc/demir_highlight()
	if(width < 1 || height < 1)
		return null

	// return_coords() gives two opposite corners in no particular order.
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

/obj/docking_port/mobile/demir_highlight()
	if(!(width && height))
		var/datum/map_template/shuttle/demir_preview/template = new
		template.port_x_offset = x
		template.port_y_offset = y
		calculate_docking_port_information(template)
	return ..()

/atom/proc/demir_highlights()
	return null

/obj/docking_port/demir_highlights()
	var/list/highlight = demir_highlight()
	return highlight ? list(highlight) : null

/datum/demir/monkestation/highlights(atom/target)
	return target.demir_highlights()

/datum/demir/monkestation/process_modular_root(list/roots)
	var/list/configs = list()
	var/list/placements = list()
	for(var/obj/modular_map_root/root as anything in roots)
		if(!root.config_file || !root.key)
			continue

		var/path = root.config_file
		if(!(path in configs))
			configs[path] = read_module_config(path)

		var/list/config = configs[path]
		if(istext(config))
			placements += list(list("root" = root, "error" = config))
			continue

		var/list/rooms = config["rooms"]
		var/list/room = rooms && rooms[root.key]
		if(!room)
			placements += list(list("root" = root, "error" = "no room [root.key] in [path]"))
			continue

		var/list/files = list()
		for(var/file in room["modules"])
			files += config["directory"] + file

		placements += list(list(
			"root" = root,
			"maps" = files,
			"anchor" = /obj/modular_map_connector,
			"skip" = list(/turf/template_noop, /area/template_noop),
		))
	demir_place_modular(placements)

/datum/demir/monkestation/proc/read_module_config(path)
	var/text = file2text(path)
	if(!text)
		return "cannot read [path]"

	try
		return demir_toml_decode(text)
	catch(var/exception/error)
		return "invalid [path]: [error.name]"

// Same DISP_DIR_* rule as the pipe's Initialize().
/datum/demir/monkestation/proc/register_disposal_node_orientations()
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

/datum/demir/monkestation
	default = TRUE
	var/smooth = TRUE
	var/lighting = TRUE
	var/show_cables = TRUE
	var/show_pipes = TRUE
	var/show_disposals = TRUE
	var/fade_underfloor = FALSE
	var/power_all = TRUE

	New()
		..()
		if(!GLOB)
			GLOB = new /datum/controller/global_vars/demir_preview

		demir_modular_root(/obj/modular_map_root)

		// Groups include subtypes, so the /visible pipes are in DEMIR_GROUP_PIPES too. Rebaking
		// them changes nothing.
		demir_define_group(DEMIR_GROUP_CABLES, /obj/structure/cable)
		demir_define_group(DEMIR_GROUP_CABLES, /obj/machinery/power/terminal)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/atmospherics/pipe)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/duct)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalpipe)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalconstruct)
		demir_define_group(DEMIR_GROUP_POWER, /obj/machinery)
		demir_define_group(DEMIR_GROUP_POWER, /turf/open/floor/circuit)
		demir_define_group(DEMIR_GROUP_POWER, /obj/item/radio/intercom)

		// Node routes avoid closed turfs. Pipes connect only on the same layer and color, so
		// supply, scrubber and stacked pipes stay separate.
		demir_node_group(/obj/structure/cable, /turf/closed, null, "cable_layer")
		demir_node_group(/obj/machinery/atmospherics/pipe/smart, DEMIR_NODE_BLOCKERS, null, list("piping_layer", "pipe_color"))
		demir_node_group(/obj/machinery/duct, DEMIR_NODE_BLOCKERS, null, list("duct_layer", "duct_color"))
		demir_node_group(/obj/structure/disposalpipe, DEMIR_NODE_BLOCKERS, /obj/structure/disposalpipe/segment)
		demir_node_group(/obj/structure/disposalconstruct, DEMIR_NODE_BLOCKERS)
		register_disposal_node_orientations()

		demir_rotatable(/obj/machinery/atmospherics, GLOB.cardinals)

// ui() writes are rolled back unless the viewer changed something that frame, so panel state
// lives in the profile's vars.
/datum/demir/monkestation/ui(atom/target)
	if(!imgui_begin("Monkestation"))
		imgui_end()
		return

	imgui_separator("Baking")
	var/smoothed = imgui_checkbox("Smooth walls", src.smooth)
	if(smoothed != src.smooth)
		src.smooth = smoothed
		// Smoothing is set on too many unrelated types to group.
		demir_rebake(DEMIR_BAKE_APPEARANCE)

	var/lit = imgui_checkbox("Lighting", src.lighting)
	if(lit != src.lighting)
		src.lighting = lit
		// Every area reads the toggle, so this rebakes all lighting.
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

	imgui_separator("Power")
	var/powered_all = imgui_checkbox("All areas powered", src.power_all)
	if(powered_all != src.power_all)
		src.power_all = powered_all
		demir_rebake(DEMIR_BAKE_LIGHT, DEMIR_GROUP_POWER)
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_POWER)

	imgui_separator("Modular maps")
	var/previewed = imgui_checkbox("Preview modules", demir_modular_preview())
	if(previewed != demir_modular_preview())
		demir_set_modular_preview(previewed)

	imgui_end()

/atom/proc/demir_prepare_state()
	demir_prepare_smoothing()

// Same as the flashlight's Initialize().
/obj/item/flashlight/demir_prepare_state()
	..()
	if(start_on)
		set_light_on(TRUE)

/turf/open/floor/light/demir_prepare_state()
	..()
	update_appearance()

// Initialize() sets the port directions from dir. Smart pipes read their neighbors' ports in
// can_be_node().
/obj/machinery/atmospherics/demir_prepare_state()
	..()
	if(pipe_flags & PIPING_CARDINAL_AUTONORMALIZE)
		normalize_cardinal_directions()
	set_init_directions(initialize_directions)

// Same as the switch's Initialize(). Its icon and light read the area it controls.
/obj/machinery/light_switch/demir_prepare_state()
	..()
	if(istext(area))
		area = text2path(area)

	if(ispath(area))
		area = GLOB.areas_by_type[area]

	if(!area)
		area = get_area(src)

/datum/demir/monkestation/prepare(atom/target)
	target.demir_prepare_state()

/atom/proc/demir_bake_smoothing()
	var/datum/demir/monkestation/profile = demir_profile()
	if(!profile.smooth || !(smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK)))
		return
	demir_smooth_icon()

/atom/proc/demir_smooth_icon()
	smooth_icon()

/turf/open/misc/grass/demir_smooth_icon()
	Initialize(TRUE)
	return ..()

/turf/open/misc/ashplanet/demir_smooth_icon()
	Initialize(TRUE)
	return ..()

/atom/proc/demir_bake_appearance()
	demir_bake_smoothing()

/obj/structure/window/demir_bake_appearance()
	demir_bake_color()
	..()
	demir_preview_window_sill()

/obj/structure/cable/demir_bake_appearance()
	Connect_cable(TRUE)
	LateInitialize()

// Multilayer cables skip the cable connection and only smooth.
/obj/structure/cable/multilayer/demir_bake_appearance()
	demir_bake_smoothing()

/obj/structure/closet/demir_bake_appearance()
	update_appearance(UPDATE_ICON)

/obj/machinery
	var/demir_calculating_light = FALSE

// power_change() also redraws the machine. Skip that during the light pass, so the appearance
// bake exports the overlays instead of keeping them as part of the mapped appearance.
/obj/machinery/update_appearance(updates = ALL)
	if(demir_calculating_light)
		return
	return ..()

/proc/demir_area_powered(area/A, chan)
	var/datum/demir/monkestation/profile = demir_profile()
	if(profile.power_all)
		return TRUE

	// When unchecked, only areas that need no power are powered.
	return A && !A.requires_power && A.powered(chan)

/obj/machinery/powered(chan = power_channel, ignore_use_power = FALSE)
	if(!loc)
		return FALSE

	if(!use_power && !ignore_use_power)
		return TRUE

	return demir_area_powered(get_area(src), chan)

/obj/machinery/proc/demir_power_change()
	power_change()
	// Subtypes set their light after calling ..() in update_appearance(), so this still runs it.
	update_appearance()

// These redraws read subsystem or reagent state that the bake does not set up.
/obj/machinery/demir_bake_appearance()
	var/static/list/skipped_types = list(
		/obj/machinery/power/solar,
		/obj/machinery/digital_clock,
		/obj/machinery/chem_master,
		/obj/machinery/stasis,
		/obj/machinery/computer/station_alert,
		/obj/machinery/barsign,
		/obj/machinery/power/smes,
		/obj/machinery/newscaster,
		/obj/machinery/portable_atmospherics/canister,
		/obj/machinery/plumbing/growing_vat,
		/obj/machinery/status_display,
	)
	if(is_type_in_list(src, skipped_types))
		return ..()

	update_appearance()
	return ..()

/obj/item/radio/intercom/demir_bake_appearance()
	on = demir_area_powered(get_area(src), AREA_USAGE_EQUIP)
	update_appearance()
	return ..()

/turf/open/floor/circuit/demir_bake_appearance()
	update_appearance()
	return ..()

/atom/proc/demir_finish_appearance()
	return

/atom/movable/demir_finish_appearance()
	demir_add_overlay_light()
	demir_apply_underfloor()

/turf/open/lava/demir_finish_appearance()
	var/datum/demir/monkestation/profile = demir_profile()
	if(!(profile.smooth && (smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK))))
		update_appearance()

/datum/demir/monkestation/bake(atom/target)
	target.demir_bake_appearance()
	target.demir_finish_appearance()
	target.demir_tag_emissive()

// The solver's falloff is (1 - (d - inner) / (range - inner)) ** curve * power, with d measured
// through demir_light_height. The values below convert LUM_FALLOFF and APPLY_CORNER to it.
/atom/proc/demir_apply_light()
	if(light_system != COMPLEX_LIGHT || !light_on || !light_outer_range || !light_power)
		return
	demir_light_range = light_outer_range
	// LUM_FALLOFF divides by max(1, outer - inner), and only that span shapes the falloff.
	demir_light_inner_range = max(0, light_outer_range - max(1, light_outer_range - light_inner_range))
	// APPLY_CORNER scales by the square of the power.
	demir_light_power = light_power ** 2 * SIGN(light_power)
	demir_light_curve = light_falloff_curve
	// LUM_FALLOFF measures the flat distance to a corner.
	demir_light_height = 0
	// PARSE_LIGHT_COLOR drops the alpha from the light color.
	var/datum/light_source/demir_preview/parsed = new
	parsed.light_color = light_color
	PARSE_LIGHT_COLOR(parsed)
	demir_light_color = rgb(parsed.lum_r * 255, parsed.lum_g * 255, parsed.lum_b * 255)

// APC load bookkeeping, unused in a bake. /area/New() would set it up, but it renames areas with
// regexes the bake cannot run.
/area/addStaticPower(value, powerchannel)
	return

// Fixtures start with `on = FALSE`, and LateInitialize() turns them on from the area's
// lightswitch and power. power_change() reads the area channel directly, so the power checkbox is
// applied here instead.
/obj/machinery/light/demir_power_change()
	var/area/local_area = get_room_area()
	on = local_area.lightswitch && demir_area_powered(local_area, AREA_USAGE_LIGHT) && status == LIGHT_OK
	update(FALSE)
	// Unpowered lights would stay on until their emergency cell drains in process().
	if(!on)
		set_light(l_outer_range = 0)

// The switch glows while its area's lights are off, so it can be found in the dark.
/obj/machinery/light_switch/demir_power_change()
	..()
	set_light(area.lightswitch ? 0 : light_on_range)

/obj/machinery/demir_apply_light()
	demir_calculating_light = TRUE
	demir_power_change()
	demir_calculating_light = FALSE
	return ..()

/turf/open/floor/circuit/demir_apply_light()
	if(!always_off && demir_area_powered(get_area(src), AREA_USAGE_LIGHT))
		on = TRUE
		set_light_color(initial(light_color))
		set_light(l_outer_range = 2, l_power = 1.5)
	else
		on = FALSE
		set_light(0)
	return ..()

// Same as the lava's Initialize(). Only lava next to another turf type casts light.
/turf/open/lava/demir_apply_light()
	refresh_light()
	// The preview has no plane offsets, so refresh_light() never checks the levels above and below.
	if(!light_on)
		for(var/turf/around as anything in block(x - 1, y - 1, z - 1, x + 1, y + 1, z + 1))
			if(!islava(around))
				set_light(l_on = TRUE)
				break
	return ..()

// LateInitialize() adds TURF_Z_TRANSPARENT_TRAIT, which lets light cross levels.
/turf/open/openspace
	demir_z_transparent = 1

/turf/open/floor/glass
	demir_z_transparent = 1

/turf/open/space/openspace
	demir_z_transparent = 1

// Space areas are fullbright. Only space tiles next to a lit turf call enable_starlight().
/turf/open/space
	demir_light_edge_only = 1

/turf/open/space/demir_apply_light()
	immediate_enable_starlight()
	return ..()

/area/demir_apply_light()
	// Turning lighting off makes the whole map fullbright instead of dark.
	var/datum/demir/monkestation/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/monkestation/light(atom/target)
	target.demir_apply_light()

// rmd only compares channel keys. The key holds the door kind and the value type, so an airlock
// id_tag never matches a poddoor id, and the number 1 never matches the text "1".
/proc/get_connection_key(kind, value)
	if(isnull(value))
		return null
	if(isnum(value))
		return "[kind]:number:[value]"
	if(istext(value))
		return "[kind]:text:[value]"
	return null

/proc/demir_connection(kind, value, roles)
	var/channel = get_connection_key(kind, value)
	var/list/connections = list()
	if(channel)
		connections[channel] = roles
	return connections

/atom/proc/demir_connections()
	return list()

/obj/machinery/button/door/demir_connections()
	if(!device)
		setup_device()
	var/obj/item/assembly/control/controller = device
	var/kind = istype(controller, /obj/item/assembly/control/airlock) ? "airlock" : "poddoor"
	return demir_connection(kind, controller?.id, DEMIR_CONNECTION_SOURCE)

/obj/machinery/door/poddoor/demir_connections()
	return demir_connection("poddoor", id, DEMIR_CONNECTION_TARGET)

/obj/machinery/door/airlock/demir_connections()
	return demir_connection("airlock", id_tag, DEMIR_CONNECTION_TARGET)

/datum/demir/monkestation/connections(atom/target)
	return target.demir_connections()

// A border object blocks only one side, but lighting corners only check whether the whole tile
// is blocked. IS_OPAQUE_TURF needs ALL_CARDINALS.
/atom/movable/demir_apply_light()
	if(opacity && (flags_1 & ON_BORDER_1))
		demir_blocks_light = 0
	return ..()

#undef DEMIR_NODE_BLOCKERS
#undef DEMIR_GROUP_CABLES
#undef DEMIR_GROUP_PIPES
#undef DEMIR_GROUP_DISPOSALS
#undef DEMIR_GROUP_UNDERFLOOR
#undef DEMIR_GROUP_POWER

#endif
