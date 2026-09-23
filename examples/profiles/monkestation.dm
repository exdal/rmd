// rmd embeds this integration for the Forced bundled profile setting. To make the profile part of
// a Monkestation checkout instead, copy it there and `#include` it from `tgstation.dme`, the guard
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

// Monkestation documents a null canSmoothWith on a smoothing atom as "smooth with the same type".
// Its bitmask helper only compares explicit groups, though, and snow mineral walls rely on the
// null form. Give those mineral walls a private type-keyed group for the preview bake.
/turf/closed/wall/mineral/demir_prepare_smoothing()
	. = ..()
	if(canSmoothWith || !(smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK)))
		return
	var/list/self_smoothing_group = list()
	self_smoothing_group[type] = TRUE
	canSmoothWith = self_smoothing_group
	smoothing_groups = smoothing_groups ? smoothing_groups.Copy() : list()
	smoothing_groups[type] = TRUE

// Use the component's own range, power, color, and directional calculations. Its New() joins
// signals and dynamic luminosity to the live game, so the preview only supplies its images.
/datum/component/overlay_lighting/demir_preview/New()
	return

/atom/movable/proc/demir_add_overlay_light()
	if(!light_on || !light_outer_range || !light_power)
		return
	if(light_system != OVERLAY_LIGHT && light_system != OVERLAY_LIGHT_DIRECTIONAL && light_system != MOVABLE_LIGHT_BEAM)
		return

	var/datum/component/overlay_lighting/demir_preview/preview = new
	preview.visible_mask = image('icons/effects/light_overlays/light_32.dmi', icon_state = "light")
	preview.visible_mask.plane = O_LIGHTING_VISUAL_PLANE
	preview.visible_mask.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
	preview.visible_mask.alpha = 0
	if(light_system == OVERLAY_LIGHT_DIRECTIONAL || light_system == MOVABLE_LIGHT_BEAM)
		preview.directional = TRUE
		preview.beam = light_system == MOVABLE_LIGHT_BEAM
		preview.cone = image('icons/effects/light_overlays/light_cone.dmi', icon_state = "light")
		preview.cone.plane = O_LIGHTING_VISUAL_PLANE
		preview.cone.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
		preview.cone.alpha = 110
		preview.cone.pixel_x = -32
		preview.cone.pixel_y = -32
		preview.set_direction(dir)

	preview.set_range(src, light_inner_range, light_outer_range)
	preview.set_power(src, light_power)
	preview.set_color(src, light_color)
	if(preview.directional)
		preview.current_holder = src
		preview.cast_directional_light()

	var/light_sign = light_power > 0 ? 1 : -1
	preview.visible_mask.demir_overlay_light = light_sign
	underlays += preview.visible_mask
	if(preview.cone)
		preview.cone.demir_overlay_light = light_sign
		underlays += preview.cone

// Flashlights normally apply their mapped start_on value during Initialize().
/obj/item/flashlight/proc/demir_prepare_light_state()
	if(start_on)
		light_on = TRUE

/turf/open/floor/light/proc/demir_prepare_light()
	// light_floor.dm undefines these constants before this profile is included:
	// fine = 0, flicker = 1, breaking = 2, broken = 3.
	if(!on || state == 3)
		light_on = FALSE
		light_outer_range = 0
		return
	light_on = TRUE
	light_color = currentcolor
	if(state == 1)
		light_outer_range = 2
	else if(state == 2)
		light_outer_range = 1
	else
		light_outer_range = 3

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

// Connect_cable() handles layers, tags, and adjacent power machinery without starting powernets.
/obj/structure/cable/proc/demir_bake_connections()
	Connect_cable(TRUE)
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

// attempt_connect() builds a live ductnet. Derive only the neighbor bits used by its native icon state.
/obj/machinery/duct/proc/demir_bake_connections()
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
			if(other_duct.dumb && !(REVERSE_DIR(check_dir) & other_duct.connects))
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
			// Polarization starts a component that requires the live game; its visual is not mapped.
			var/polarizer_id = spawner.polarizer_id
			spawner.polarizer_id = ""
			spawner.Initialize(TRUE)
			spawner.polarizer_id = polarizer_id
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
	if(plane != FLOOR_PLANE)
		plane = FLOOR_PLANE

/obj/machinery/door/airlock/proc/demir_bake_appearance()
	if(glass)
		airlock_material = "glass"
	// Initialize() normally gives the native overlay builder a string suffix.
	if(isnull(fill_state_suffix))
		fill_state_suffix = ""
	update_appearance(UPDATE_ICON)

// set_light() still selects Monkestation's own light color, power, and range.
// The bake's lighting pass consumes those fields; no game light_source is needed.
/obj/machinery/door/airlock/update_light()
	return

// A mapped mobile port has no loaded shuttle template. Give the native sizing proc the edited
// map as its preview template, then let return_coords() handle every orientation.
/datum/map_template/shuttle/demir_preview/New()
	return

/obj/docking_port/mobile/proc/demir_prepare_preview_bounds()
	if(width && height)
		return
	var/datum/map_template/shuttle/demir_preview/preview = new
	preview.width = world.maxx
	preview.height = world.maxy
	preview.port_x_offset = x
	preview.port_y_offset = y
	calculate_docking_port_information(preview)

/obj/docking_port/proc/demir_highlight()
	if(istype(src, /obj/docking_port/mobile))
		var/obj/docking_port/mobile/mobile = src
		mobile.demir_prepare_preview_bounds()
	if(width < 1 || height < 1)
		return null
	var/list/corners = return_coords()
	return list(
		"x" = min(corners[1], corners[3]) - x,
		"y" = min(corners[2], corners[4]) - y,
		"width" = abs(corners[3] - corners[1]) + 1,
		"height" = abs(corners[4] - corners[2]) + 1,
		"color" = "#ff8000",
		"when" = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED,
		"label" = name || "docking port",
	)

/datum/demir/monkestation/highlights(atom/target)
	if(!istype(target, /obj/docking_port))
		return null

	var/obj/docking_port/port = target
	var/list/highlight = port.demir_highlight()

	return highlight ? list(highlight) : null

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
		demir_node_group(/obj/structure/disposalpipe, DEMIR_NODE_BLOCKERS)
		demir_node_group(/obj/structure/disposalconstruct, DEMIR_NODE_BLOCKERS)

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

/datum/demir/monkestation/prepare(atom/target)
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

/datum/demir/monkestation/bake(atom/target)
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
	else if(src.smooth && (target.smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK)))
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
	if(light_system != COMPLEX_LIGHT || !light_on || !light_outer_range || !light_power)
		return
	demir_light_range = max(light_outer_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_inner_range = min(light_inner_range, demir_light_range)
	demir_light_power = light_power
	demir_light_color = light_color
	demir_light_curve = light_falloff_curve

// Fixtures ship with `on = FALSE` and are switched on by area power during
// post_machine_initialize(), which reaches reagents, SSmachines and prob().
/obj/machinery/light/demir_apply_light()
	if(status == LIGHT_OK)
		on = TRUE
		light_on = TRUE
		light_outer_range = bulb_outer_range
		light_inner_range = bulb_inner_range
		light_power = bulb_power
		light_color = color || bulb_colour
		light_falloff_curve = bulb_falloff
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

/datum/demir/monkestation/connections(atom/target)
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
