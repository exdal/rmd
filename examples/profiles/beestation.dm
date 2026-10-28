// rmd embeds this profile for the Forced bundled profile setting. To make the profile part of a
// BeeStation checkout instead, copy it there and `#include` it from `beestation.dme` after
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

/datum/controller/subsystem/icon_smooth/demir_preview/New()
	SSicon_smooth = src

/datum/controller/subsystem/vis_overlays/demir_preview/New()
	SSvis_overlays = src
	vis_overlay_cache = list()
	unique_vis_overlays = list()
	mob_alpha_vis_overlays = list()

/datum/controller/subsystem/overlays/demir_preview/New()
	SSoverlays = src
	stats = list()

/datum/controller/subsystem/processing/dcs/demir_preview/New()
	SSdcs = src

// Elements work through signals, which do not run in a bake.
/datum/controller/subsystem/processing/dcs/demir_preview/GetElement(list/arguments)
	return null

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
	if(!SSdcs)
		SSdcs = new /datum/controller/subsystem/processing/dcs/demir_preview
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
	if(!SSicon_smooth)
		SSicon_smooth = new /datum/controller/subsystem/icon_smooth/demir_preview
	if(!SSvis_overlays)
		SSvis_overlays = new /datum/controller/subsystem/vis_overlays/demir_preview
	bitflag_lists = list()
	gvars_datum_init_order = list()
	InitGlobalcardinals()
	InitGlobaldiagonals()
	InitGlobalalldirs()
	InitGlobalpipe_color_name()
	InitGlobalcameranet()
	InitGlobalemissive_color()
	InitGlobalem_block_color()
	InitGlobalareas()
	InitGlobalareas_by_type()
	InitGlobalstation_levels_cache()

// Same as the end of /atom/Initialize(), minus the sorted-list asserts.
/atom/proc/demir_prepare_smoothing()
	if(length(smoothing_groups))
		SET_BITFLAG_LIST(smoothing_groups)

	if(length(canSmoothWith))
		if(canSmoothWith[length(canSmoothWith)] > MAX_S_TURF)
			smoothing_flags |= SMOOTH_OBJ

		SET_BITFLAG_LIST(canSmoothWith)

	if(uses_integrity)
		atom_integrity = max_integrity

// Same light_system switch as the end of /atom/movable/Initialize().
/atom/movable/proc/demir_add_overlay_light()
	var/datum/demir/beestation/profile = demir_profile()
	if(!profile.movable_lights)
		return

	switch(light_system)
		if(MOVABLE_LIGHT)
			AddComponent(/datum/component/overlay_lighting)
		if(MOVABLE_LIGHT_DIRECTIONAL)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE)
		if(MOVABLE_LIGHT_BEAM)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE, is_beam = TRUE)

// The light hook replaces /datum/light_source, so set_light() only stores its values on the
// atom.
/atom/update_light()
	return

// filter() is not available to a bake, and the editor draws no filters.
/atom/update_filters()
	return

// 1 for an emissive mask, -1 for an emissive blocker. The game draws both with a color matrix.
/proc/demir_emissive_role(color)
	if(!islist(color))
		return 0

	var/list/color_matrix = color
	if(color_matrix[17] || color_matrix[18])
		return 1

	if(color_matrix[16] && !color_matrix[19])
		return -1

	return 0

/image/proc/demir_tag_emissive()
	if(plane == EMISSIVE_PLANE)
		var/role = demir_emissive_role(color)
		if(role > 0)
			demir_emissive = TRUE
		else if(role < 0)
			demir_emissive_blocker = TRUE
	// The plane master cuts whatever sits on this plane out of the darkness.
	else if(plane == O_LIGHTING_VISUAL_PLANE)
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
	if(plane == EMISSIVE_PLANE)
		var/role = demir_emissive_role(color)
		if(role > 0)
			demir_emissive = TRUE
		else if(role < 0)
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

// BEESTATION FIX YOUR SHIT: open closets pass themselves as the layer of their door's emissive
// blocker, which is not a number.
/proc/emissive_blocker(icon, icon_state = "", layer = FLOAT_LAYER, alpha = 255, appearance_flags = NONE)
	if(!isnum(layer))
		layer = FLOAT_LAYER

	var/mutable_appearance/appearance = mutable_appearance(icon, icon_state, layer, EMISSIVE_PLANE, alpha, appearance_flags | EMISSIVE_APPEARANCE_FLAGS)
	appearance.color = GLOB.em_block_color
	return appearance

// update_appearance() redraws gas_output, which Initialize() links.
/obj/machinery/atmospherics/gasrig/core/demir_bake_appearance()
	update_icon()

/obj/machinery/atmospherics/pipe/layer_manifold/demir_bake_appearance()
	icon_state = "manifoldlayer_center"
	return ..()

/obj/machinery/atmospherics/pipe/multiz/demir_bake_appearance()
	icon_state = ""
	center = mutable_appearance(icon, "adapter_center", layer = HIGH_OBJ_LAYER)
	pipe = mutable_appearance(icon, "pipe-[piping_layer]")
	return ..()

// Connecting redraws every duct it links, and a bake exports each neighbor it changed. The
// neighbors would export an appearance that knows only this duct, so only the baked duct redraws.
/obj/machinery/duct
	var/demir_is_baking = FALSE

/obj/machinery/duct/update_icon()
	if(!demir_is_baking)
		return
	return ..()

/obj/machinery/duct/demir_bake_appearance()
	demir_is_baking = TRUE
	attempt_connect()
	demir_is_baking = FALSE

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
		if(spawned.smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK))
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
	var/datum/demir/beestation/profile = demir_profile()
	return profile.show_cables

/obj/machinery/power/terminal/demir_underfloor_shown()
	var/datum/demir/beestation/profile = demir_profile()
	return profile.show_cables

// setup_hiding() runs only when hide is set, so the /visible subtypes never take the element.
/obj/machinery/atmospherics/pipe/demir_underfloor_shown()
	var/datum/demir/beestation/profile = demir_profile()
	return hide ? profile.show_pipes : null

/obj/machinery/duct/demir_underfloor_shown()
	var/datum/demir/beestation/profile = demir_profile()
	return profile.show_pipes

/obj/structure/disposalpipe/demir_underfloor_shown()
	var/datum/demir/beestation/profile = demir_profile()
	return profile.show_disposals

/obj/structure/disposalconstruct/demir_underfloor_shown()
	var/datum/demir/beestation/profile = demir_profile()
	return profile.show_disposals

/atom/movable/proc/demir_apply_underfloor()
	var/shown = demir_underfloor_shown()
	if(isnull(shown))
		return

	var/turf/our_turf = loc
	if(!isturf(our_turf) || our_turf.underfloor_accessibility >= UNDERFLOOR_INTERACTABLE)
		return

	var/datum/demir/beestation/profile = demir_profile()
	if(our_turf.underfloor_accessibility < UNDERFLOOR_VISIBLE)
		if(!shown)
			alpha = 0
			return

		if(profile.fade_underfloor)
			// ALPHA_UNDERTILE is undefined at the end of undertile.dm.
			alpha = 128

	// The element moves what it leaves visible to FLOOR_PLANE, so pipes draw under the machines on
	// their tile. A pipe inside a wall keeps its plane, or the wall would cover it.
	if(plane != FLOOR_PLANE && !isclosedturf(our_turf))
		plane = FLOOR_PLANE

// The door body is blank. Its two halves are airlock_part effects, and its frame and lights are vis
// overlays. A bake does not export vis_contents, so they go out as overlays. The game draws nothing
// for a state its icon lacks, but the editor draws its missing icon, so those layers and the blank
// body are left out.
/obj/machinery/door/airlock/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	rebuild_parts()
	update_appearance(UPDATE_ICON)
	for(var/obj/effect/overlay/airlock_part/part as anything in part_overlays)
		var/list/drawn = list()
		for(var/image/layer in part.overlays)
			if(layer.icon_state in icon_states(layer.icon))
				drawn += layer

		part.overlays = drawn
		overlays += part

	for(var/obj/effect/overlay/vis/layer as anything in managed_vis_overlays)
		if(layer.icon_state in icon_states(layer.icon))
			overlays += layer

	icon = null
	icon_state = null

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

	var/datum/demir/beestation/profile = demir_profile()
	var/when = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED
	// Only seen tiles stay on, or the blind tiles of nearby cameras would cover them.
	var/seen_when = profile.show_camera_ranges ? when | DEMIR_HIGHLIGHT_ALWAYS : when
	return list(
		list("tiles" = seen, "color" = "#40c040", "fill" = 0.25, "when" = seen_when),
		list("tiles" = blind, "color" = "#e04040", "fill" = 0.25, "when" = when),
		list("x" = 0, "y" = 0, "width" = 1, "height" = 1, "fill" = 0, "outline" = 0, "when" = when, "label" = jointext(network, ", ")),
	)

/datum/demir/beestation/highlights(atom/target)
	return target.demir_highlights()

// dpdir comes from Initialize() and depends only on the pipe's dir and initialize_dirs.
/datum/demir/beestation/proc/register_disposal_node_orientations()
	var/list/openings_by_dirs = list()
	for(var/type_path in typesof(/obj/structure/disposalpipe))
		var/obj/structure/disposalpipe/pipe_type = type_path
		var/extra = "[initial(pipe_type.initialize_dirs)]"
		if(!openings_by_dirs[extra])
			var/list/openings = list()
			for(var/direction in GLOB.alldirs)
				var/obj/structure/disposalpipe/pipe = new type_path
				pipe.setDir(direction)
				pipe.Initialize(TRUE)
				openings["[direction]"] = pipe.dpdir

			openings_by_dirs[extra] = openings

		var/list/openings = openings_by_dirs[extra]
		for(var/direction in GLOB.alldirs)
			demir_node_orientation(type_path, direction, openings["[direction]"])

/datum/demir/beestation
	default = TRUE
	var/smooth = TRUE
	var/lighting = TRUE
	var/show_cables = TRUE
	var/show_pipes = TRUE
	var/show_disposals = TRUE
	var/fade_underfloor = FALSE
	var/show_camera_ranges = FALSE
	var/power_all = TRUE
	var/movable_lights = TRUE

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
		demir_node_group(/obj/structure/cable, /turf/closed, null, "cable_color")
		demir_node_group(/obj/machinery/atmospherics/pipe/smart, DEMIR_NODE_BLOCKERS, null, list("piping_layer", "pipe_color"))
		demir_node_group(/obj/machinery/duct, DEMIR_NODE_BLOCKERS, null, list("duct_layer", "duct_color"))
		demir_node_group(/obj/structure/disposalpipe, DEMIR_NODE_BLOCKERS, /obj/structure/disposalpipe/segment)
		demir_node_group(/obj/structure/disposalconstruct, DEMIR_NODE_BLOCKERS)
		register_disposal_node_orientations()

		demir_rotatable(/obj/machinery/atmospherics, GLOB.cardinals)

// ui() writes are rolled back unless the viewer changed something that frame, so panel state
// lives in the profile's vars.
/datum/demir/beestation/ui(atom/target)
	if(!imgui_begin("beestation"))
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

	var/movable = imgui_checkbox("Movable lights", src.movable_lights)
	if(movable != src.movable_lights)
		src.movable_lights = movable
		// Flashlights, lamps and other movable lights are set on too many types to group.
		demir_rebake(DEMIR_BAKE_APPEARANCE)

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
		update_appearance(UPDATE_ICON)

// Same as the fixture's Initialize() and its deferred spawn(). The roundstart chance to break a
// bulb is left out, so the preview does not flicker between bakes.
/obj/machinery/light/demir_prepare_state()
	..()
	var/area/local_area = get_area(src)
	var/is_bulb = fitting == "bulb"
	if(bulb_colour == initial(bulb_colour))
		bulb_colour = is_bulb ? local_area.lighting_colour_bulb : local_area.lighting_colour_tube

	brightness = is_bulb ? local_area.lighting_brightness_bulb : local_area.lighting_brightness_tube

/turf/open/floor/light/demir_prepare_state()
	..()
	update_appearance()

// BEESTATION FIX YOUR SHIT: `j << 1` writes to the number instead of shifting it, which a bake does
// not allow. The loop only sums the volume, so this keeps the ports.
/obj/machinery/atmospherics/pipe/smart/set_init_directions(init_dir)
	initialize_directions = init_dir || ALL_CARDINALS

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

// The preset calls upgradeXRay() in Initialize(), which needs the camera's assembly.
/obj/machinery/camera/demir_prepare_state()
	..()
	if(istype(src, /obj/machinery/camera/xray))
		upgrades |= CAMERA_UPGRADE_XRAY

/datum/demir/beestation/prepare(atom/target)
	target.demir_prepare_state()

/atom/proc/demir_bake_smoothing()
	var/datum/demir/beestation/profile = demir_profile()
	if(!profile.smooth || !(smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK)))
		return
	smooth_icon()

/atom/proc/demir_bake_appearance()
	demir_bake_smoothing()

// reform_connections() redraws every neighbor it links, and a bake exports each neighbor it changed.
// The neighbors would export an appearance that knows only this cable, so only the baked cable
// redraws. linkup_adjacent() builds the powernet and is left out.
/obj/structure/cable
	var/demir_is_baking = FALSE

/obj/structure/cable/update_appearance(updates = ALL)
	if(!demir_is_baking)
		return
	return ..()

/obj/structure/cable/demir_bake_appearance()
	demir_is_baking = TRUE
	reform_connections()
	update_appearance(UPDATE_ICON)
	demir_is_baking = FALSE

/obj/structure/closet/demir_bake_appearance()
	update_appearance(UPDATE_ICON)

/obj/machinery/light/demir_bake_appearance()
	var/datum/demir/beestation/profile = demir_profile()
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
	var/datum/demir/beestation/profile = demir_profile()
	if(profile.power_all)
		return TRUE
	// When unchecked, only areas that need no power are powered.
	return A && !A.requires_power && A.powered(chan)

/obj/machinery/powered(chan = power_channel, ignore_use_power = FALSE)
	if(!use_power && !ignore_use_power)
		return TRUE
	return demir_area_powered(get_area(src), chan)

/obj/machinery/proc/demir_power_change()
	// Their power_change() starts sound loops, trackers and subsystem work the bake does not set up.
	var/static/list/unsafe_types = list(
		/obj/machinery/status_display,
		/obj/machinery/gravity_generator,
		/obj/machinery/porta_turret,
		/obj/machinery/telecomms,
		/obj/machinery/stasis,
		/obj/machinery/atmospherics/gasrig,
		/obj/machinery/airalarm,
	)
	if(is_type_in_list(src, unsafe_types))
		machine_stat = powered(power_channel) ? machine_stat & ~NOPOWER : machine_stat | NOPOWER
		return

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
		/obj/machinery/stasis,
		/obj/machinery/computer/station_alert,
		/obj/machinery/status_display,
		/obj/machinery/oven,
		/obj/machinery/jukebox,
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
	var/datum/demir/beestation/profile = demir_profile()
	if(!(profile.smooth && (smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK))))
		update_appearance()

/datum/demir/beestation/bake(atom/target)
	target.demir_bake_appearance()
	target.demir_finish_appearance()
	target.demir_tag_emissive()

/atom/proc/demir_apply_light()
	if(light_system != STATIC_LIGHT || !light_on || !light_range || !light_power)
		return
	demir_light_range = max(light_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_power = light_power
	demir_light_color = light_color
	demir_light_height = light_height

/obj/machinery/demir_apply_light()
	demir_calculating_light = TRUE
	demir_power_change()
	demir_calculating_light = FALSE
	return ..()

// Fixtures start with `on = FALSE`. power_change() turns them on from the area's lightswitch and
// power, then update() waits on a timer and rolls for burning out. The power checkbox is applied
// here instead, and the fixture is lit the way turn_on() would light it.
/obj/machinery/light/demir_power_change()
	var/area/local_area = get_area(src)
	on = local_area.lightswitch && demir_area_powered(local_area, AREA_USAGE_LIGHT) && status == LIGHT_OK
	if(!on)
		set_light(0)
		return

	set_light(brightness, bulb_power, color || bulb_colour)

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
	return ..()

// Initialize() adds TURF_Z_TRANSPARENT_TRAIT, which lets light cross levels.
/turf/open/openspace
	demir_z_transparent = 1

/turf/open/floor/glass
	demir_z_transparent = 1

// Space areas are fullbright. Space tiles light their neighbors with the light_* vars they carry,
// but only along the edge of the fullbright region.
/turf/open/space
	demir_light_edge_only = 1

// Turfs with a fullbright type never get a lighting object, whatever their area does.
/turf/demir_apply_light()
	demir_fullbright = fullbright_type != FULLBRIGHT_NONE
	return ..()

/area/demir_apply_light()
	// Turning lighting off makes the whole map fullbright instead of dark.
	var/datum/demir/beestation/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/beestation/light(atom/target)
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

/datum/demir/beestation/connections(atom/target)
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
