// rmd embeds this profile for the Forced bundled profile setting. To keep it in a TGMC checkout
// instead, copy it there and `#include` it from `tgmc.dme` after `// END_INCLUDE`. BYOND skips it
// because of the guard below.
//
// TGMC has tgstation's bitflag-list smoothing and atmos, with static, hybrid and movable lights.
// The hooks set up only the vars the icon code reads and start no subsystems.
#ifdef __DEMIR_BAKE__

#define DEMIR_GROUP_CABLES (1<<0)
#define DEMIR_GROUP_PIPES (1<<1)
#define DEMIR_GROUP_DISPOSALS (1<<2)
#define DEMIR_GROUP_WEEDS (1<<3)
#define DEMIR_GROUP_CAMERAS (1<<4)
#define DEMIR_GROUP_POWER (1<<5)
#define DEMIR_GROUP_UNDERFLOOR (DEMIR_GROUP_CABLES | DEMIR_GROUP_PIPES | DEMIR_GROUP_DISPOSALS)

// The bake has no plane offsets, so planes stay unshifted.
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

/datum/controller/subsystem/icon_smooth/demir_preview/New()
	SSicon_smooth = src

/datum/controller/subsystem/icon_smooth/demir_preview/add_to_queue(atom/target)
	return

/datum/controller/global_vars/demir_preview/New()
	GLOB = src
	gvars_datum_init_order = list()
	bitflag_lists = list()
	InitGlobalcardinals()
	InitGlobaldiagonals()
	InitGlobalalldirs()
	InitGlobalwire_node_generating_types()
	InitGlobaltopdown_planes()
	InitGlobalemissive_color_no_bloom()
	InitGlobalemissive_color()
	InitGlobalem_block_color()
	InitGlobalweed_prob_list()
	if(!SSmapping)
		SSmapping = new /datum/controller/subsystem/mapping/demir_preview
	if(!SSoverlays)
		SSoverlays = new /datum/controller/subsystem/overlays/demir_preview
	if(!SSatoms)
		SSatoms = new /datum/controller/subsystem/atoms/demir_preview
	if(!SSicon_smooth)
		SSicon_smooth = new /datum/controller/subsystem/icon_smooth/demir_preview

/datum/demir/tgmc
	default = TRUE
	var/smooth = TRUE
	var/lighting = TRUE
	var/show_cables = TRUE
	var/show_pipes = TRUE
	var/show_disposals = TRUE
	var/fade_underfloor = FALSE
	var/show_camera_ranges = FALSE
	var/show_weeds = FALSE
	var/power_all = TRUE

	New()
		..()
		if(!GLOB)
			GLOB = new /datum/controller/global_vars/demir_preview
		demir_define_group(DEMIR_GROUP_CABLES, /obj/structure/cable)
		demir_define_group(DEMIR_GROUP_CABLES, /obj/machinery/power/terminal)
		demir_define_group(DEMIR_GROUP_PIPES, /obj/machinery/atmospherics)
		demir_define_group(DEMIR_GROUP_DISPOSALS, /obj/structure/disposalpipe)
		demir_define_group(DEMIR_GROUP_CAMERAS, /obj/machinery/camera)
		demir_define_group(DEMIR_GROUP_WEEDS, /obj/effect/landmark/weed_node)
		demir_define_group(DEMIR_GROUP_POWER, /obj/machinery)
		demir_node_group(/obj/structure/cable, /turf/closed, null, "cable_layer")
		demir_node_group(/obj/machinery/atmospherics/pipe, /turf/closed, /obj/machinery/atmospherics/pipe/simple, list("piping_layer", "pipe_color"))
		demir_node_group(/obj/structure/disposalpipe, /turf/closed, /obj/structure/disposalpipe/segment)
		for(var/direction in GLOB.cardinals)
			var/openings = direction | REVERSE_DIR(direction)
			demir_node_orientation(/obj/machinery/atmospherics/pipe/simple, direction, openings)
		for(var/direction in GLOB.diagonals)
			demir_node_orientation(/obj/machinery/atmospherics/pipe/simple, direction, direction)
		register_disposal_node_orientations()
		demir_rotatable(/obj/machinery/atmospherics, GLOB.cardinals)

// Same openings as the Initialize() of each pipe type. Most set dpdir inline.
/datum/demir/tgmc/proc/register_disposal_node_orientations()
	for(var/type_path in typesof(/obj/structure/disposalpipe))
		var/obj/structure/disposalpipe/pipe_type = type_path
		var/state = initial(pipe_type.icon_state)
		for(var/direction in GLOB.cardinals)
			var/openings = 0
			if(ispath(type_path, /obj/structure/disposalpipe/segment))
				openings = direction | (state == "pipe-s" ? REVERSE_DIR(direction) : turn(direction, -90))
			else if(ispath(type_path, /obj/structure/disposalpipe/junction))
				switch(state)
					if("pipe-j1")
						openings = direction | turn(direction, -90) | REVERSE_DIR(direction)
					if("pipe-j2")
						openings = direction | turn(direction, 90) | REVERSE_DIR(direction)
					else
						openings = direction | turn(direction, 90) | turn(direction, -90)
			else if(ispath(type_path, /obj/structure/disposalpipe/sortjunction))
				openings = direction | REVERSE_DIR(direction) | turn(direction, state == "pipe-j1s" ? -90 : 90)
			else if(ispath(type_path, /obj/structure/disposalpipe/tagger))
				openings = direction | REVERSE_DIR(direction)
			else if(ispath(type_path, /obj/structure/disposalpipe/trunk) || ispath(type_path, /obj/structure/disposalpipe/up) || ispath(type_path, /obj/structure/disposalpipe/down))
				openings = direction
			demir_node_orientation(type_path, direction, openings)

/datum/demir/tgmc/ui(atom/target)
	if(!imgui_begin("TGMC"))
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
		// Every area reads the toggle, and movable lights add masks to appearances.
		demir_rebake(DEMIR_BAKE_LIGHT)
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

	imgui_separator("Weeds")
	var/weeds = imgui_checkbox("Fully grown", src.show_weeds)
	if(weeds != src.show_weeds)
		src.show_weeds = weeds
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_WEEDS)

	imgui_separator("Power")
	var/powered_all = imgui_checkbox("All areas powered", src.power_all)
	if(powered_all != src.power_all)
		src.power_all = powered_all
		demir_rebake(DEMIR_BAKE_LIGHT, DEMIR_GROUP_POWER)
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_POWER)

	imgui_end()

// Initialize() packs the smoothing groups into shared bitflag lists.
/atom/proc/demir_prepare_state()
	if(length(smoothing_groups))
		sortTim(smoothing_groups)
		SET_BITFLAG_LIST(smoothing_groups)
	if(length(canSmoothWith))
		sortTim(canSmoothWith)
		if(canSmoothWith[length(canSmoothWith)] > MAX_S_TURF)
			smoothing_flags |= SMOOTH_OBJ
		SET_BITFLAG_LIST(canSmoothWith)

/turf/closed/wall/demir_prepare_state()
	..()
	if(isnull(wall_integrity))
		wall_integrity = max_integrity

/obj/machinery/atmospherics/demir_prepare_state()
	..()
	if(pipe_flags & PIPING_CARDINAL_AUTONORMALIZE)
		normalize_cardinal_directions()
	SetInitDirections()

/obj/machinery/door/poddoor/demir_prepare_state()
	..()
	opacity = density

/datum/demir/tgmc/prepare(atom/target)
	target.demir_prepare_state()

// The bake computes the lightmap, so set_light() only stores its values on the atom.
/atom/update_light()
	return

// TGMC declares this proc without a body, so give it an explicit one.
/atom/update_icon_state()
	return

/proc/demir_area_powered(area/target, channel)
	var/datum/demir/tgmc/profile = demir_profile()
	if(profile.power_all)
		return TRUE
	return target && !target.requires_power

/obj/machinery/powered(chan = -1)
	if(!loc)
		return FALSE
	if(use_power == NO_POWER_USE)
		return TRUE
	return demir_area_powered(get_area(src), chan == -1 ? power_channel : chan)

/atom/proc/demir_apply_light()
	if((light_system != STATIC_LIGHT && light_system != HYBRID_LIGHT) || !light_range || !light_power)
		return
	demir_light_range = max(light_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_power = light_power
	demir_light_color = light_color
	demir_light_height = LIGHTING_HEIGHT
	// Same as UPDATE_APPROXIMATE_PIXEL_TURF, which moves the source a tile for large pixel offsets.
	var/turf/origin = get_turf(src)
	var/offset = GET_APPROXIMATE_PIXEL_DIR(pixel_x, pixel_y)
	var/turf/source_turf = offset ? (get_step(origin, offset) || origin) : origin
	if(origin && source_turf)
		demir_light_offset_x = source_turf.x - origin.x
		demir_light_offset_y = source_turf.y - origin.y

/obj/machinery/demir_apply_light()
	if(powered())
		machine_stat &= ~NOPOWER
	else
		machine_stat |= NOPOWER
	return ..()

// update() reads live area power and can burn out the bulb, so set its end result directly.
/obj/machinery/light/demir_apply_light()
	var/area/local_area = get_area(src)
	light_on = local_area && local_area.lightswitch && powered(LIGHT) && status == LIGHT_OK
	light_range = light_on ? brightness : 0
	light_power = bulb_power
	light_color = color ? color : bulb_colour
	update_offsets()
	return ..()

/area/demir_apply_light()
	var/datum/demir/tgmc/profile = demir_profile()
	demir_fullbright = !profile.lighting || !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/turf/open/transparent
	demir_z_transparent = TRUE

/datum/demir/tgmc/light(atom/target)
	target.demir_apply_light()

/atom/proc/demir_bake_smoothing()
	var/datum/demir/tgmc/profile = demir_profile()
	if(profile.smooth && (smoothing_flags & (SMOOTH_CORNERS | SMOOTH_BITMASK)))
		smooth_icon()

/atom/proc/demir_bake_appearance()
	demir_bake_smoothing()

/obj/machinery/demir_bake_appearance()
	update_appearance()
	return ..()

// The vat overlay reads reagents that only exist in a live round.
/obj/machinery/cloning/vats/demir_bake_appearance()
	return demir_bake_smoothing()

/obj/machinery/light/demir_bake_appearance()
	var/datum/demir/tgmc/profile = demir_profile()
	if(profile.lighting)
		update_appearance()
	return demir_bake_smoothing()

/obj/machinery/atmospherics/demir_bake_appearance()
	nodes = list()
	nodes.len = device_type
	atmosinit()

/obj/structure/cable
	var/demir_is_bake_target = FALSE

// Connect_cable() also redraws every linked neighbor. The bake would draw that half-linked redraw
// over the neighbor's own, so only the cable being baked redraws.
/obj/structure/cable/update_icon(updates)
	if(!demir_is_bake_target)
		return
	return ..()

/obj/structure/cable/demir_bake_appearance()
	demir_is_bake_target = TRUE
	Connect_cable(TRUE)

/obj/structure/cable/multilayer/demir_bake_appearance()
	demir_is_bake_target = TRUE
	update_icon()

/obj/structure/closet/demir_bake_appearance()
	update_appearance()

/obj/item/flashlight/demir_bake_appearance()
	update_icon()

/obj/effect/landmark/weed_node/demir_bake_appearance()
	var/datum/demir/tgmc/profile = demir_profile()
	if(!profile.show_weeds)
		return ..()

	var/node_type = pickweight(GLOB.weed_prob_list)
	var/obj/alien/weeds/node/node_path = node_type
	var/color_variant = initial(node_path.color_variant)
	var/node_icon = initial(node_path.node_icon)
	var/list/cluster = demir_weed_cluster()
	var/list/owners = demir_weed_owners(cluster)
	var/list/grown = demir_weed_grown(cluster, owners)

	invisibility = 0
	icon = 'icons/Xeno/weeds.dmi'
	icon_state = demir_weed_floor_state(loc, grown, color_variant)
	layer = XENO_WEEDS_LAYER
	plane = FLOOR_PLANE
	var/image/node_overlay = new
	node_overlay.icon = icon
	node_overlay.icon_state = "[node_icon][rand(0, 5)]"
	overlays += node_overlay

	for(var/turf/T as anything in grown)
		var/is_owner = owners[T] == src
		if(!is_owner || T == loc)
			continue

		var/image/weed = demir_weed_image(T, grown, color_variant)
		weed.pixel_x = (T.x - x) * world.icon_size
		weed.pixel_y = (T.y - y) * world.icon_size
		overlays += weed

/obj/effect/landmark/weed_node/proc/demir_weed_cluster()
	var/list/cluster = list()
	var/turf/low = locate(max(1, x - 5), max(1, y - 5), z)
	var/turf/high = locate(min(world.maxx, x + 5), min(world.maxy, y + 5), z)
	for(var/turf/T as anything in block(low, high))
		for(var/obj/effect/landmark/weed_node/node in T)
			cluster += node

	return cluster

/obj/effect/landmark/weed_node/proc/demir_weed_owners(list/cluster)
	var/list/owners = list()
	for(var/obj/effect/landmark/weed_node/node as anything in cluster)
		for(var/turf/T as anything in demir_weed_pending(node))
			var/obj/effect/landmark/weed_node/current = owners[T]
			if(current && !demir_weed_is_closer(node, current, T))
				continue

			owners[T] = node

	return owners

/proc/demir_weed_pending(obj/effect/landmark/weed_node/node)
	var/list/pending = list()
	var/turf/origin = node.loc
	pending[origin] = TRUE
	var/list/frontier = list(origin)
	for(var/step in 1 to 2)
		var/list/next = list()
		for(var/turf/T as anything in frontier)
			for(var/direction in GLOB.alldirs)
				var/turf/neighbor = get_step(T, direction)
				if(!neighbor || pending[neighbor])
					continue

				pending[neighbor] = TRUE
				if(!neighbor.density)
					next += neighbor

		frontier = next

	return pending

/proc/demir_weed_is_closer(obj/effect/landmark/weed_node/node, obj/effect/landmark/weed_node/current, turf/T)
	var/node_distance = get_dist_euclidean_square(node, T)
	var/current_distance = get_dist_euclidean_square(current, T)
	if(node_distance != current_distance)
		return node_distance < current_distance

	if(node.y != current.y)
		return node.y < current.y

	return node.x < current.x

/obj/effect/landmark/weed_node/proc/demir_weed_grown(list/cluster, list/owners)
	var/list/grown = list()
	var/list/frontier = list()
	for(var/obj/effect/landmark/weed_node/node as anything in cluster)
		var/turf/node_turf = node.loc
		if(grown[node_turf])
			continue

		grown[node_turf] = TRUE
		frontier += node_turf

	for(var/depth in 1 to 5)
		var/list/next = list()
		for(var/turf/T as anything in frontier)
			for(var/direction in GLOB.cardinals)
				var/turf/neighbor = get_step(T, direction)
				if(!neighbor || grown[neighbor] || !owners[neighbor] || !neighbor.is_weedable())
					continue

				var/obj/machinery/door/blocker = locate(/obj/machinery/door) in neighbor
				if(blocker && blocker.density)
					continue

				grown[neighbor] = TRUE
				next += neighbor

		frontier = next

	return grown

/proc/demir_weed_floor_state(turf/T, list/grown, color_variant)
	var/mask = 0
	for(var/direction in GLOB.cardinals)
		var/turf/neighbor = get_step(T, direction)
		if(!neighbor)
			continue

		if(grown[neighbor] || istype(neighbor, /turf/closed/wall/resin))
			mask |= direction

	if(mask == 15)
		return "weed[rand(0, 15)][color_variant]"

	if(!mask)
		return "base[color_variant]"

	return "weed_dir[mask][color_variant]"

/proc/demir_weed_image(turf/T, list/grown, color_variant)
	var/image/weed = new
	var/atom/wall = null
	if(iswallturf(T))
		wall = T
		weed.layer = WEEDWALL_LAYER
	else
		wall = locate(/obj/structure/window/framed) in T
		if(!wall)
			wall = locate(/obj/structure/window_frame) in T
		weed.layer = GIB_LAYER

	if(!wall)
		weed.icon = 'icons/Xeno/weeds.dmi'
		weed.icon_state = demir_weed_floor_state(T, grown, color_variant)
		weed.layer = XENO_WEEDS_LAYER
		weed.plane = FLOOR_PLANE
		return weed

	wall.demir_bake_smoothing()
	var/junction = wall.smoothing_junction
	weed.icon_state = junction ? "weedwall-[junction]" : "weedwall"
	weed.plane = GAME_PLANE
	switch(color_variant)
		if(STICKY_COLOR)
			weed.icon = 'icons/obj/smooth_objects/weedwallsticky.dmi'
		if(RESTING_COLOR)
			weed.icon = 'icons/obj/smooth_objects/weedwallrest.dmi'
		else
			weed.icon = 'icons/obj/smooth_objects/weedwall.dmi'

	return weed

/atom/movable/proc/demir_underfloor_shown()
	return null

/obj/structure/cable/demir_underfloor_shown()
	var/datum/demir/tgmc/profile = demir_profile()
	return profile.show_cables

/obj/machinery/power/terminal/demir_underfloor_shown()
	var/datum/demir/tgmc/profile = demir_profile()
	return profile.show_cables

/obj/machinery/atmospherics/pipe/demir_underfloor_shown()
	var/datum/demir/tgmc/profile = demir_profile()
	return level == 1 ? profile.show_pipes : null

/obj/structure/disposalpipe/demir_underfloor_shown()
	var/datum/demir/tgmc/profile = demir_profile()
	return profile.show_disposals

/atom/movable/proc/demir_apply_underfloor()
	var/shown = demir_underfloor_shown()
	if(isnull(shown))
		return
	var/turf/local_turf = loc
	if(!isturf(local_turf) || !local_turf.intact_tile)
		return
	var/datum/demir/tgmc/profile = demir_profile()
	invisibility = 0
	alpha = !shown ? 0 : (profile.fade_underfloor ? 128 : 255)

/atom/proc/demir_finish_appearance()
	return

/atom/movable/demir_finish_appearance()
	demir_add_overlay_light()
	demir_apply_underfloor()

/datum/demir/tgmc/bake(atom/target)
	target.demir_bake_appearance()
	target.demir_finish_appearance()
	target.demir_tag_emissive()

// The mask /datum/component/overlay_lighting draws, without its signals or holder setup.
/atom/movable/proc/demir_add_overlay_light()
	var/datum/demir/tgmc/profile = demir_profile()
	if(!profile.lighting || light_system != MOVABLE_LIGHT || !light_on || !light_range || !light_power)
		return
	var/range = clamp(CEILING(light_range, 0.5), 1, 6)
	var/list/light_overlays = /datum/component/overlay_lighting::light_overlays
	var/image/mask = new
	mask.icon = light_overlays["[((range - 1) * 64) + 32]"]
	mask.icon_state = "light"
	mask.plane = O_LIGHTING_VISUAL_PLANE
	mask.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
	mask.alpha = min(230, (abs(light_power) * 120) + 30)
	mask.color = light_color
	mask.demir_overlay_light = light_power < 0 ? -1 : 1
	mask.pixel_x = light_pixel_x - ((range - 1) * 32)
	mask.pixel_y = light_pixel_y - ((range - 1) * 32)
	underlays += mask

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

	var/datum/demir/tgmc/profile = demir_profile()
	var/when = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED
	// Only seen tiles stay on, or the blind tiles of nearby cameras would cover them.
	var/seen_when = profile.show_camera_ranges ? when | DEMIR_HIGHLIGHT_ALWAYS : when
	return list(
		list("tiles" = seen, "color" = "#40c040", "fill" = 0.25, "when" = seen_when),
		list("tiles" = blind, "color" = "#e04040", "fill" = 0.25, "when" = when),
		list("x" = 0, "y" = 0, "width" = 1, "height" = 1, "fill" = 0, "outline" = 0, "when" = when, "label" = jointext(network, ", ")),
	)

/datum/demir/tgmc/highlights(atom/target)
	return target.demir_highlights()

// The key holds the value type, so the number 1 never matches the text "1".
/proc/demir_connection(kind, value, roles)
	var/list/result = list()
	if(isnum(value))
		result["[kind]:number:[value]"] = roles
	else if(istext(value))
		result["[kind]:text:[value]"] = roles
	return result

/atom/proc/demir_connections()
	return null

/obj/machinery/door_control/demir_connections()
	return demir_connection(normaldoorcontrol ? "airlock" : "poddoor", id, DEMIR_CONNECTION_SOURCE)

/obj/machinery/door/poddoor/demir_connections()
	return demir_connection("poddoor", id, DEMIR_CONNECTION_TARGET)

/obj/machinery/door/airlock/demir_connections()
	return demir_connection("airlock", id_tag, DEMIR_CONNECTION_TARGET)

/datum/demir/tgmc/connections(atom/target)
	return target.demir_connections()

#undef DEMIR_GROUP_CABLES
#undef DEMIR_GROUP_PIPES
#undef DEMIR_GROUP_DISPOSALS
#undef DEMIR_GROUP_WEEDS
#undef DEMIR_GROUP_CAMERAS
#undef DEMIR_GROUP_POWER
#undef DEMIR_GROUP_UNDERFLOOR

#endif
