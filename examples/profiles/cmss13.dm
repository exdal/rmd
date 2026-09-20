// A template, not something rmd loads on its own. Copy this file into a cmss13 checkout and
// `#include` it from `colonialmarines.dme`; the guard keeps BYOND compiling it to nothing.
//
// Walls pick corner states from their neighbours in LateInitialize(), and windows, frames and a
// few floors pick a junction state through relativewall(). Lights follow TGMC's three systems:
// static and hybrid lights are corner lights here, and movable lights are masks.
#ifdef __DEMIR_BAKE__

/datum/controller/global_vars/demir_preview/New()
	GLOB = src
	gvars_datum_init_order = list()
	InitGlobalcardinals()
	InitGlobaldiagonals()
	InitGlobalalldirs()

/datum/demir/cmss13
	default = TRUE

	New()
		..()
		if(!GLOB)
			GLOB = new /datum/controller/global_vars/demir_preview

// Walls fill a shared damage overlay cache on first draw. Filling it here keeps it out of every
// rolled-back preview.
/turf/closed/wall/proc/demir_prepare_overlays()
	if(!damage_overlays[1])
		generate_damage_overlays()

/atom/proc/demir_prepare_state()
	return

// `set_light_on(on)` from Initialize()
/obj/item/device/flashlight/demir_prepare_state()
	light_on = on

// power_change() switches fixtures from the area a tick after they load. APCs drain and refill
// the area's channels later still, so the mapped area vars stand in for them.
/obj/structure/machinery/light/demir_prepare_state()
	var/area/fixture_area = get_area(src)
	if(!fixture_area)
		return
	var/switched = fixture_area.lightswitch
	if(needs_power && !fixture_area.unlimited_power)
		switched = switched && fixture_area.power_light
	// LIGHT_OK is 0, and lightreplacer.dm undefines it before this profile is included.
	on = switched && status == 0
	light_range = on ? brightness : 0

// update_icon() colours the screen by charge state and only lights a closed, working APC.
// Round start has not charged anything yet. APC_NOT_CHARGING is 0 and apc.dm undefines it.
/obj/structure/machinery/power/apc/demir_prepare_state()
	if(stat & (BROKEN|MAINT) || !cell_type)
		light_range = 0
		return
	light_color = charging == 0 ? LIGHT_COLOR_RED : LIGHT_COLOR_GREEN

// `Initialize()` makes an open poddoor see-through before update_icon() shows it open
/obj/structure/machinery/door/poddoor/demir_prepare_state()
	opacity = density

/datum/demir/cmss13/prepare(atom/target)
	if(istype(target, /turf/closed/wall))
		var/turf/closed/wall/wall = target
		wall.demir_prepare_overlays()
	target.demir_prepare_state()

/proc/demir_overlay_icon(pixel_bounds)
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
		if(384)
			return 'icons/effects/light_overlays/light_384.dmi'
		if(416)
			return 'icons/effects/light_overlays/light_416.dmi'
	return null

// The overlay lighting component's mask and cone, without components, signals or lumcounts.
/atom/movable/proc/demir_add_overlay_light()
	if((light_system != MOVABLE_LIGHT && light_system != DIRECTIONAL_LIGHT) || !light_on || !light_range)
		return

	var/range = clamp(CEILING(light_range, 0.5), 1, 7)
	var/pixel_bounds = ((range - 1) * 64) + 32
	var/image/mask = new
	mask.icon = demir_overlay_icon(pixel_bounds)
	mask.icon_state = "light"
	mask.plane = O_LIGHTING_VISUAL_PLANE
	mask.appearance_flags = RESET_COLOR | RESET_ALPHA | RESET_TRANSFORM
	mask.alpha = min(230, (abs(light_power) * 120) + 30)
	mask.color = light_color
	mask.demir_overlay_light = light_power < 0 ? -1 : 1
	mask.pixel_x = -((range - 1) * 32)
	mask.pixel_y = mask.pixel_x

	if(light_system == DIRECTIONAL_LIGHT)
		// `cast_directional_light()`, with SHORT_CAST at 2
		var/final_distance = clamp(floor(light_range * 0.5), 1, 3)
		if(final_distance > 2 && !(ALL_CARDINALS & dir))
			final_distance -= 1
		var/turf/scanning = get_turf(src)
		for(var/i in 1 to final_distance)
			var/turf/next_turf = get_step(scanning, dir)
			if(isnull(next_turf) || next_turf.opacity)
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
		cone.alpha = min(200, (abs(light_power) * 90) + 20)
		cone.color = light_color
		cone.pixel_x = -32
		cone.pixel_y = -32
		cone.demir_overlay_light = mask.demir_overlay_light
		underlays += cone

	underlays += mask

/atom/proc/demir_bake_icon()
	return

/turf/closed/wall/demir_bake_icon()
	update_connections(FALSE)
	update_icon()

/obj/structure/machinery/light/demir_bake_icon()
	update_icon()

/obj/structure/machinery/door/poddoor/demir_bake_icon()
	update_icon()

/obj/structure/machinery/door/airlock/demir_bake_icon()
	update_icon()

/obj/item/device/flashlight/demir_bake_icon()
	update_icon()

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
	for(var/image/underlay in underlays)
		underlay.demir_tag_emissive()
	for(var/image/overlay in overlays)
		overlay.demir_tag_emissive()

/datum/demir/cmss13/bake(atom/target)
	target.demir_bake_icon()
	if(target.tiles_with && !istype(target, /turf/closed/wall))
		target.relativewall()
	if(ismovable(target))
		var/atom/movable/movable_target = target
		movable_target.demir_add_overlay_light()
	target.demir_tag_emissive()

// Initialize() hands every light that is neither movable nor directional to update_light(), which
// builds a static source or a hybrid mask source. Neither checks light_on.
/atom/proc/demir_apply_light()
	if(light_system == MOVABLE_LIGHT || light_system == DIRECTIONAL_LIGHT || !light_range || !light_power)
		return
	demir_light_range = max(light_range, MINIMUM_USEFUL_LIGHT_RANGE)
	demir_light_power = light_power
	demir_light_color = light_color
	demir_light_height = LIGHTING_HEIGHT

/area/demir_apply_light()
	demir_fullbright = !static_lighting
	demir_ambient_color = base_lighting_color
	demir_ambient_power = base_lighting_alpha / 255

/datum/demir/cmss13/light(atom/target)
	target.demir_apply_light()

#endif
