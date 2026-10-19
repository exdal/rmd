// rmd embeds this profile for the Forced bundled profile setting. To make the profile part of a
// Nova Sector checkout instead, copy it there and `#include` it from `tgstation.dme` after
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
#define DEMIR_GROUP_LIGHTS (1<<3)
#define DEMIR_GROUP_CAMERAS (1<<4)
#define DEMIR_GROUP_POWER (1<<5)
#define DEMIR_GROUP_UNDERFLOOR (DEMIR_GROUP_CABLES | DEMIR_GROUP_PIPES | DEMIR_GROUP_DISPOSALS)

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

// Same light_system switch as the end of /atom/movable/Initialize().
/atom/movable/proc/demir_add_overlay_light()
	switch(light_system)
		if(OVERLAY_LIGHT)
			AddComponent(/datum/component/overlay_lighting)
		if(OVERLAY_LIGHT_DIRECTIONAL)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE)
		if(OVERLAY_LIGHT_BEAM)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE, is_beam = TRUE)

// Power changes can queue an autoclose. Keep doors as mapped.
/obj/machinery/door/autoclose_in(wait)
	return

// The light hook replaces /datum/light_source, so set_light() only stores its values on the
// atom.
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

// TGSTATION FIX YOUR SHIT: the ternary takes `FLOAT_LAYER - HAS_TRAIT(...)` as its condition, leaving every
// connection on absolute layer 1, under the floor once a hidden manifold sinks onto FLOOR_PLANE.
/obj/machinery/atmospherics/pipe/layer_manifold/get_attached_image(p_dir, p_layer, p_color)
	var/mutable_appearance/connection = ..()
	connection.layer = FLOAT_LAYER - (HAS_TRAIT(src, TRAIT_UNDERFLOOR) ? 1 : 0.01)
	return connection

/obj/machinery/atmospherics/pipe/multiz/demir_bake_appearance()
	icon_state = ""
	center = mutable_appearance(icon, "adapter_center", layer = HIGH_OBJ_LAYER)
	pipe = mutable_appearance(icon, "pipe-[piping_layer]")
	return ..()

// post_machine_initialize() links both ducts and redraws the neighbor. The bake only exports the
// duct it runs on, so this copies the color and layer checks and leaves the neighbor alone.
/obj/machinery/duct/demir_bake_appearance()
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

// /datum/element/undertile hides cables and pipes on COMSIG_OBJ_HIDE. Signals do not run in a
// bake, so this reads the turf's underfloor_accessibility instead. Null means the element never
// applies to the type.
/atom/movable/proc/demir_underfloor_shown()
	return null

/obj/structure/cable/demir_underfloor_shown()
	var/datum/demir/novasector/profile = demir_profile()
	return profile.show_cables

/obj/machinery/power/terminal/demir_underfloor_shown()
	var/datum/demir/novasector/profile = demir_profile()
	return profile.show_cables

// setup_hiding() runs only when hide is set, so the /visible subtypes never take the element.
/obj/machinery/atmospherics/pipe/demir_underfloor_shown()
	var/datum/demir/novasector/profile = demir_profile()
	return hide ? profile.show_pipes : null

/obj/machinery/duct/demir_underfloor_shown()
	var/datum/demir/novasector/profile = demir_profile()
	return profile.show_pipes

/obj/structure/disposalpipe/demir_underfloor_shown()
	var/datum/demir/novasector/profile = demir_profile()
	return profile.show_disposals

/obj/structure/disposalconstruct/demir_underfloor_shown()
	var/datum/demir/novasector/profile = demir_profile()
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

	var/datum/demir/novasector/profile = demir_profile()
	if(profile.fade_underfloor)
		// ALPHA_UNDERTILE is undefined at the end of undertile.dm.
		alpha = 128

	// The element moves what it leaves visible to FLOOR_PLANE, so pipes draw under the machines on
	// their tile. Cables already there keep the layer offsets they sort by. SET_PLANE_IMPLICIT is
	// skipped because the bake has no multi-z plane offsets. A pipe inside a wall keeps its plane,
	// or the wall would cover it.
	if(plane != FLOOR_PLANE && !isclosedturf(our_turf))
		plane = FLOOR_PLANE
		layer = BELOW_CATWALK_LAYER

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

/obj/machinery/camera/demir_highlights()
	var/turf/origin = get_turf(src)
	if(!origin)
		return null

	var/list/visible = list()
	if(can_use())
		for(var/turf/T in can_see())
			visible[T] = TRUE

	var/list/seen = list()
	var/list/blind = list()
	for(var/turf/T as anything in block(origin.x - view_range, origin.y - view_range, origin.z, origin.x + view_range, origin.y + view_range))
		if(visible[T])
			seen += list(list(T.x - origin.x, T.y - origin.y))
		else
			blind += list(list(T.x - origin.x, T.y - origin.y))

	var/datum/demir/novasector/profile = demir_profile()
	var/when = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED
	// Only seen tiles stay on, or the blind tiles of nearby cameras would cover them.
	var/seen_when = profile.show_camera_ranges ? when | DEMIR_HIGHLIGHT_ALWAYS : when
	return list(
		list("tiles" = seen, "color" = "#40c040", "fill" = 0.25, "when" = seen_when),
		list("tiles" = blind, "color" = "#e04040", "fill" = 0.25, "when" = when),
		list("x" = 0, "y" = 0, "width" = 1, "height" = 1, "fill" = 0, "outline" = 0, "when" = when, "label" = jointext(network, ", ")),
	)

/datum/demir/novasector/highlights(atom/target)
	return target.demir_highlights()

// Same DISP_DIR_* rule as the pipe's Initialize().
/datum/demir/novasector/proc/register_disposal_node_orientations()
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

/datum/demir/novasector
	default = TRUE
	var/smooth = TRUE
	var/lighting = TRUE
	var/show_cables = TRUE
	var/show_pipes = TRUE
	var/show_disposals = TRUE
	var/fade_underfloor = FALSE
	var/show_camera_ranges = FALSE
	var/power_all = TRUE

	New()
		..()
		if(!GLOB)
			GLOB = new /datum/controller/global_vars/demir_preview

		// Groups include subtypes, so the /visible pipes are in DEMIR_GROUP_PIPES too. Rebaking
		// them changes nothing.
		demir_define_group(DEMIR_GROUP_CABLES, /obj/structure/cable)
		demir_define_group(DEMIR_GROUP_CABLES, /obj/machinery/power/terminal)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/atmospherics/pipe)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/duct)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalpipe)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalconstruct)
		demir_define_group(DEMIR_GROUP_LIGHTS, /obj/machinery/light)
		demir_define_group(DEMIR_GROUP_CAMERAS, /obj/machinery/camera)
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
/datum/demir/novasector/ui(atom/target)
	if(!imgui_begin("Nova Sector"))
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
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_LIGHTS)

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

	imgui_separator("Cameras")
	var/camera_ranges = imgui_checkbox("Show all ranges", src.show_camera_ranges)
	if(camera_ranges != src.show_camera_ranges)
		src.show_camera_ranges = camera_ranges
		demir_rebake(DEMIR_BAKE_HIGHLIGHT, DEMIR_GROUP_CAMERAS)

	imgui_separator("Power")
	var/powered_all = imgui_checkbox("All areas powered", src.power_all)
	if(powered_all != src.power_all)
		src.power_all = powered_all
		demir_rebake(DEMIR_BAKE_LIGHT, DEMIR_GROUP_POWER)
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_POWER)

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

// The bake skips New(), where areas set up their power bookkeeping.
/area/demir_prepare_state()
	New()
	return ..()

// Same as the switch's Initialize(). Its icon and light read the area it controls.
/obj/machinery/light_switch/demir_prepare_state()
	..()
	if(istext(area))
		area = text2path(area)

	if(ispath(area))
		area = GLOB.areas_by_type[area]

	if(!area)
		area = get_area(src)

// Nova air alarms read the area that Initialize() caches.
/obj/machinery/airalarm/demir_prepare_state()
	..()
	my_area = get_area(src)

// The presets call upgradeXRay() in Initialize(). It also builds a module and updates SScameras.
/obj/machinery/camera/demir_prepare_state()
	..()
	if(istype(src, /obj/machinery/camera/xray) || istype(src, /obj/machinery/camera/all))
		camera_upgrade_bitflags |= CAMERA_UPGRADE_XRAY

/datum/demir/novasector/prepare(atom/target)
	target.demir_prepare_state()

/atom/proc/demir_bake_smoothing()
	var/datum/demir/novasector/profile = demir_profile()
	if(!profile.smooth || !(smoothing_flags & USES_SMOOTHING))
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

/obj/structure/cable/demir_bake_appearance()
	connect_cable(TRUE)
	LateInitialize()

// Multilayer cables skip the cable connection and only smooth.
/obj/structure/cable/multilayer/demir_bake_appearance()
	demir_bake_smoothing()

/obj/structure/closet/demir_bake_appearance()
	update_appearance(UPDATE_ICON)

/obj/machinery/light/demir_bake_appearance()
	var/datum/demir/novasector/profile = demir_profile()
	if(profile.lighting)
		update_appearance(UPDATE_ICON)
	return demir_bake_smoothing()

/obj/machinery
	var/demir_calculating_light = FALSE

// power_change() also redraws the machine. Skip that during the light pass, so the appearance
// bake exports the overlays instead of keeping them as part of the mapped appearance.
/obj/machinery/update_appearance(updates = ALL)
	if(demir_calculating_light)
		return
	return ..()

/proc/demir_area_powered(area/A, chan)
	var/datum/demir/novasector/profile = demir_profile()
	if(profile.power_all)
		return TRUE
	// When unchecked, only areas that need no power are powered.
	return A && !A.requires_power && A.powered(chan)

/obj/machinery/powered(chan = power_channel, ignore_use_power = FALSE)
	if(!use_power && !ignore_use_power)
		return TRUE
	return demir_area_powered(get_area(src), chan)

// Their power_change() reads live shuttle or round state, so they keep the mapped appearance.
/obj/machinery/status_display/demir_power_change()
	return

/obj/machinery/gravity_generator/main/demir_power_change()
	return

/obj/machinery/proc/demir_power_change()
	power_change()
	// Subtypes set their light after calling ..() in update_appearance(), so this still runs it.
	update_appearance()

// These redraws read subsystem or reagent state that the bake does not set up.
/obj/machinery/demir_bake_appearance()
	var/static/list/skipped_types = list(
		/obj/machinery/power/solar,
		/obj/machinery/power/smes,
		/obj/machinery/newscaster,
		/obj/machinery/portable_atmospherics/canister,
		/obj/machinery/digital_clock,
		/obj/machinery/chem_master,
		/obj/machinery/chem_mass_spec,
		/obj/machinery/vatgrower,
		/obj/machinery/stasis,
		/obj/machinery/computer/station_alert,
		/obj/machinery/status_display,
		/obj/machinery/exoscanner,
		/obj/machinery/barsign,
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

/obj/structure/extinguisher_cabinet/demir_bake_appearance()
	update_appearance(UPDATE_ICON)

/atom/proc/demir_finish_appearance()
	return

/atom/movable/demir_finish_appearance()
	demir_add_overlay_light()
	demir_apply_underfloor()

/turf/open/lava/demir_finish_appearance()
	var/datum/demir/novasector/profile = demir_profile()
	if(!(profile.smooth && (smoothing_flags & USES_SMOOTHING)))
		update_appearance()

/datum/demir/novasector/bake(atom/target)
	target.demir_bake_appearance()
	target.demir_finish_appearance()
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
	// The game adds these offsets to the sample position, which moves the source the opposite way.
	demir_light_offset_x = -light_offset[1]
	demir_light_offset_y = -light_offset[2]
	demir_light_height = light_height

/obj/machinery/demir_apply_light()
	demir_calculating_light = TRUE
	demir_power_change()
	demir_calculating_light = FALSE
	return ..()

// Fixtures start with `on = FALSE`. Initialize() aims the light with setDir(), and
// post_machine_initialize() turns it on from the area's lightswitch and power. power_change()
// reads the area channel directly, so the power checkbox is applied here instead.
/obj/machinery/light/demir_apply_light()
	setDir(dir)
	return ..()

/obj/machinery/light/demir_power_change()
	var/area/local_area = get_room_area()
	on = local_area.lightswitch && demir_area_powered(local_area, AREA_USAGE_LIGHT) && status == LIGHT_OK
	// update() counts every switch toward burning out the bulb, so reset the count each bake.
	switchcount = 0
	// Nova turns lights on after a timer. Skip it and show the end state.
	update(FALSE, instant = TRUE, play_sound = FALSE)
	// Unpowered lights would stay on until their emergency cell drains in process().
	if(!on)
		set_light(l_range = 0)

// The switch glows while its area's lights are off, so it can be found in the dark.
/obj/machinery/light_switch/demir_power_change()
	..()
	set_light(area.lightswitch ? 0 : light_on_range)

/turf/open/floor/circuit/demir_apply_light()
	if(!always_off && demir_area_powered(get_area(src), AREA_USAGE_LIGHT))
		on = TRUE
		set_light_color(initial(light_color))
		set_light(2, 1.5)
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

// Initialize() adds TURF_Z_TRANSPARENT_TRAIT, which lets light cross levels.
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
	demir_light_range = GLOB.starlight_range
	demir_light_power = GLOB.starlight_power
	demir_light_color = GLOB.starlight_color

/area/demir_apply_light()
	// Turning lighting off makes the whole map fullbright instead of dark.
	var/datum/demir/novasector/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/novasector/light(atom/target)
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
		setup_device(TRUE)
	var/obj/item/assembly/control/controller = device
	var/kind = istype(controller, /obj/item/assembly/control/airlock) ? "airlock" : "poddoor"
	return demir_connection(kind, controller?.id, DEMIR_CONNECTION_SOURCE)

/obj/machinery/door/poddoor/demir_connections()
	return demir_connection("poddoor", id, DEMIR_CONNECTION_TARGET)

/obj/machinery/door/airlock/demir_connections()
	return demir_connection("airlock", id_tag, DEMIR_CONNECTION_TARGET)

/datum/demir/novasector/connections(atom/target)
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
#undef DEMIR_GROUP_LIGHTS
#undef DEMIR_GROUP_CAMERAS
#undef DEMIR_GROUP_POWER
#undef DEMIR_GROUP_UNDERFLOOR

#endif
