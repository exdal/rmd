// rmd embeds this integration for the Forced bundled profile setting. To make the profile part of
// a Monkestation checkout instead, copy it there and `#include` it from `tgstation.dme`, the guard
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

/datum/controller/subsystem/machines/demir_preview/New()
	SSmachines = src

/datum/controller/subsystem/icon_smooth/demir_preview/New()
	SSicon_smooth = src

// The bake smooths every atom itself.
/datum/controller/subsystem/icon_smooth/demir_preview/add_to_queue(atom/thing)
	return

// Initialize() builds the department palette and recolors the windows already placed in each
// area. The bake colors one window at a time, so this keeps the palette by area type instead.
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

// The light_system switch that closes /atom/movable/Initialize(). The component draws its own
// mask and cone.
/atom/movable/proc/demir_add_overlay_light()
	switch(light_system)
		if(OVERLAY_LIGHT)
			AddComponent(/datum/component/overlay_lighting)
		if(OVERLAY_LIGHT_DIRECTIONAL)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE)
		if(MOVABLE_LIGHT_BEAM)
			AddComponent(/datum/component/overlay_lighting, is_directional = TRUE, is_beam = TRUE)

// Overlay lights add to the dynamic_lumcount mobs read to count the light they stand in, through a
// view() of the live map.
/datum/component/overlay_lighting/get_new_turfs()
	return

// The lighting subsystem's /datum/light_source is what the light hook stands in for, so the
// codebase's own set_light*() calls only leave their values on the atom.
/atom/update_light()
	return

// PARSE_LIGHT_COLOR reads a light source, whose New() joins the lighting subsystem.
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
		// The plane master lights whatever it masks, whatever the image's blend mode.
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

// Link this device's nodes with the codebase's own connection checks; atmos_init()
// then runs its native update_appearance() without starting pipe networks.
/obj/machinery/atmospherics/demir_bake_appearance()
	// GAGS sheets are generated at runtime, so keep the prebuilt map preview state.
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

// connect_duct() builds a live ductnet as it links, so this mirrors its checks and keeps only the
// neighbor bits the native icon state reads.
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

// Inline in the window's Initialize(), which leaves a sill under every fulltile window it colors.
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

// The sill smooths against its neighbors', so preview theirs too and keep only the one on this
// turf. It is its own object in the game, so it never takes the window's tint.
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

// Structure window spawners are mapping helpers. Preview the grille and windows
// their native Initialize() creates without committing those temporary atoms.
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

// Cables and pipes are covered by a floor tile through /datum/element/undertile, which reacts to
// COMSIG_OBJ_HIDE from levelupdate(). No signals or elements run here, so read the turf's own
// accessibility instead and apply the same result.
// Null for anything the element never covers, so a shown one is told apart from an untouched one.
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
		// undertile.dm undefines its own ALPHA_UNDERTILE before this profile is included.
		alpha = 128

	// The element sinks whatever it leaves visible onto the floor plane, and only what is not already
	// there. Without it a pipe covers the machines standing on its own tile; with the guard a cable
	// keeps the per-cable-layer offsets it draws its ordering from. Assigned rather than
	// SET_PLANE_IMPLICIT because the multi-z plane offsets that consults mean nothing to a bake.
	// A wall's plane covers the floor plane, so a pipe running through one keeps its own plane.
	if(plane != FLOOR_PLANE && !isclosedturf(our_turf))
		plane = FLOOR_PLANE

// Inline in the airlock's Initialize().
/obj/machinery/door/airlock/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	update_appearance(UPDATE_ICON)

// A mapped mobile port is sized when its shuttle template loads. The template is the map being
// edited, so its bounds stand in for the file preload_size() would parse.
/datum/map_template/shuttle/demir_preview/preload_size(path, cache)
	width = world.maxx
	height = world.maxy

/obj/docking_port/proc/demir_highlight()
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

// The DISP_DIR_* rule is inline in the pipe's Initialize().
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

// ui() rolls its writes back on every frame but the one the viewer touched something on, so the
// profile's own vars are where panel state belongs. Every other hook reads them off src, and the
// frame that changes one re-derives appearances, highlights and lighting.
/datum/demir/monkestation
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
		// every route out of closed turfs. Placements connect only on the same layer and
		// color, so adjacent supply, scrubber and stacked pipes never merge.
		demir_node_group(/obj/structure/cable, /turf/closed, null, "cable_layer")
		demir_node_group(/obj/machinery/atmospherics/pipe/smart, DEMIR_NODE_BLOCKERS, null, list("piping_layer", "pipe_color"))
		demir_node_group(/obj/machinery/duct, DEMIR_NODE_BLOCKERS, null, list("duct_layer", "duct_color"))
		demir_node_group(/obj/structure/disposalpipe, DEMIR_NODE_BLOCKERS, /obj/structure/disposalpipe/segment)
		demir_node_group(/obj/structure/disposalconstruct, DEMIR_NODE_BLOCKERS)
		register_disposal_node_orientations()

		demir_rotatable(/obj/machinery/atmospherics, GLOB.cardinals)

/datum/demir/monkestation/ui(atom/target)
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

/atom/proc/demir_prepare_state()
	demir_prepare_smoothing()

// Inline in the flashlight's Initialize().
/obj/item/flashlight/demir_prepare_state()
	..()
	if(start_on)
		set_light_on(TRUE)

/turf/open/floor/light/demir_prepare_state()
	..()
	update_appearance()

// Atmos Initialize() normally derives each port from dir before atmos_init().
// Smart pipes need those neighboring port directions for can_be_node().
/obj/machinery/atmospherics/demir_prepare_state()
	..()
	if(pipe_flags & PIPING_CARDINAL_AUTONORMALIZE)
		normalize_cardinal_directions()
	set_init_directions(initialize_directions)

/obj/structure/closet/demir_prepare_state()
	..()
	PopulateContents()

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

// Inline in the computer's Initialize(), and its first call draws the powered or unpowered
// keyboard and screen.
/obj/machinery/computer/demir_bake_appearance()
	power_change()
	return ..()

/obj/structure/window/demir_bake_appearance()
	demir_bake_color()
	..()
	demir_preview_window_sill()

/obj/structure/cable/demir_bake_appearance()
	Connect_cable(TRUE)
	LateInitialize()

// Multilayer cables bypass the ordinary cable setup, but retain the default smoothing path.
/obj/structure/cable/multilayer/demir_bake_appearance()
	demir_bake_smoothing()

/obj/structure/closet/demir_bake_appearance()
	update_appearance(UPDATE_ICON)

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
// through demir_light_height. Each value below turns LUM_FALLOFF and APPLY_CORNER into those terms.
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
	// PARSE_LIGHT_COLOR drops the alpha a light color spells.
	var/datum/light_source/demir_preview/parsed = new
	parsed.light_color = light_color
	PARSE_LIGHT_COLOR(parsed)
	demir_light_color = rgb(parsed.lum_r * 255, parsed.lum_g * 255, parsed.lum_b * 255)

// The APC's load bookkeeping, which /area/New() sets up and the preview never reads. That New()
// renames areas through regex matching the bake cannot run.
/area/addStaticPower(value, powerchannel)
	return

// Fixtures ship with `on = FALSE`, and post_machine_initialize() switches them on from the area's
// lightswitch and power.
/obj/machinery/light/demir_apply_light()
	power_change()
	return ..()

// Inline in the lava's Initialize(). Only lava bordering another turf casts light.
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

// Space is a fullbright area, so its tiles never hold a lighting object and only the ones
// touching a statically lit turf call enable_starlight().
/turf/open/space
	demir_light_edge_only = 1

/turf/open/space/demir_apply_light()
	immediate_enable_starlight()
	return ..()

/area/demir_apply_light()
	// Switching lighting off lights the whole map rather than blacking it out, which is what a
	// mapper wants from the toggle.
	var/datum/demir/monkestation/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/monkestation/light(atom/target)
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

#endif
