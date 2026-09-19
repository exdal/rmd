// A template, not something rmd loads on its own. Copy this file into a Goonstation checkout and
// `#include` it from `goonstation.dme`; the guard keeps BYOND compiling it to nothing.
//
// Goonstation derives its icons in `UpdateIcon()` and keeps its per-type data in a typeinfo datum,
// so the profile resolves that datum per atom and lets the codebase pick its own icon state.
// Mapped atoms never run New(), so the state UpdateIcon() reads and the lights New() attaches are
// set up here instead.
#ifdef __DEMIR_BAKE__

/atom/proc/demir_prepare_state()
	return

// `New()`
/turf/simulated/wall/auto/asteroid/demir_prepare_state()
	LAZYLISTINIT(topoverlaycache)
	space_overlays = list()
	topnumber = pick(1, 2, 3)
	orenumber = pick(1, 2, 3)
	color = stone_color

// `New()`
/obj/machinery/firealarm/demir_prepare_state()
	alarm_base_overlay = image(icon, src, "fireoff")
	alarm_overlay = image(icon, src, "fireoff")
	alarm_overlay.plane = PLANE_LIGHTING
	alarm_overlay.blend_mode = BLEND_ADD
	alarm_overlay.layer = LIGHTING_LAYER_BASE
	alarm_overlay.alpha = 80

// `New()`
/obj/machinery/portable_atmospherics/canister/demir_prepare_state()
	atmos_dmi = image('icons/obj/atmospherics/atmos.dmi')
	bomb_dmi = image('icons/obj/canisterbomb.dmi')

/datum/demir/goonstation/prepare(atom/target)
	target.get_typeinfo()
	target.demir_prepare_state()
	target.demir_prepare_lights()

/atom/proc/demir_bake_extras()
	return

/turf/simulated/wall/auto/asteroid/demir_bake_extras()
	space_overlays()

// The rock texture on top is cut to the wall shape by an alpha filter the editor cannot draw. The
// smoothed base state already carries that shape.
/turf/simulated/wall/auto/asteroid/top_overlays()
	return

/turf/simulated/wall/auto/asteroid/ore_overlays()
	if(ore)
		var/image/ore_overlay = mutable_appearance('icons/turf/walls/asteroid.dmi', "[ore.name][orenumber]")
		ore_overlay.layer = ASTEROID_ORE_OVERLAY_LAYER
		AddOverlays(ore_overlay, "ast_ore")

// Starlight only shows where it lands on something that is not already fullbright, and open space
// would otherwise add one sprite per tile.
/turf/space/demir_bake_extras()
	if(!length(underlays))
		return
	for(var/direction in alldirs)
		var/turf/neighbor = get_step(src, direction)
		if(neighbor && !neighbor.fullbright)
			return
	underlays = list()

// The screen glow New() builds and power_change() adds. The 0.33 greyscale matrix it uses is
// flattened to its brightness.
/atom/proc/demir_add_screen_glow(glow_icon, glow_state)
	var/image/screen = image(glow_icon, glow_state, -1)
	screen.plane = PLANE_LIGHTING
	screen.blend_mode = BLEND_ADD
	screen.layer = LIGHTING_LAYER_BASE
	screen.color = rgb(84, 84, 84)
	AddOverlays(screen, "screen_image")

/obj/machinery/computer/demir_bake_extras()
	if(glow_in_dark_screen && !(status & BROKEN) && powered())
		demir_add_screen_glow('icons/obj/computer_screens.dmi', icon_state)

/obj/machinery/computer3/demir_bake_extras()
	if(glow_in_dark_screen && !(status & BROKEN) && powered())
		demir_add_screen_glow('icons/obj/computer_screens.dmi', icon_state)

/obj/machinery/disposal/chemlink/demir_bake_extras()
	if(powered())
		demir_add_screen_glow('icons/obj/disposal.dmi', "chemlink_screen")

// SimpleLight's vis_contents objects, as images the bake can export.
/atom/proc/demir_simple_light(list/rgba)
	var/image/light = new
	light.icon = 'icons/effects/overlays/simplelight.dmi'
	light.icon_state = "3x3"
	light.plane = PLANE_LIGHTING
	light.layer = LIGHTING_LAYER_BASE
	light.blend_mode = BLEND_ADD
	light.appearance_flags = RESET_COLOR | RESET_TRANSFORM | RESET_ALPHA | NO_CLIENT_COLOR | KEEP_APART
	light.pixel_x = -32
	light.pixel_y = -32
	light.color = rgb(rgba[1], rgba[2], rgba[3], rgba[4])
	overlays += light

/obj/machinery/bot/secbot/demir_bake_extras()
	demir_simple_light(list(255, 255, 255, 0.4 * 255))

/obj/machinery/bot/cleanbot/demir_bake_extras()
	demir_simple_light(list(255, 255, 255, 0.4 * 255))

/obj/machinery/bot/medbot/demir_bake_extras()
	demir_simple_light(list(220, 220, 255, 0.5 * 255))

/obj/machinery/bot/guardbot/demir_bake_extras()
	if(on)
		demir_simple_light(list(flashlight_red * 255, flashlight_green * 255, flashlight_blue * 255, (flashlight_lum / 7) * 255))

/obj/naval_mine/demir_bake_extras()
	demir_simple_light(list(255, 102, 102, 40))

// Anything drawn on the lighting plane adds to the light there, and subtracting blends take it away.
/image/proc/demir_tag_light()
	if(plane == PLANE_LIGHTING)
		if(blend_mode == BLEND_SUBTRACT)
			demir_overlay_light = -1
		else if(blend_mode != BLEND_MULTIPLY)
			demir_overlay_light = 1
	for(var/image/underlay in underlays)
		underlay.demir_tag_light()
	for(var/image/overlay in overlays)
		overlay.demir_tag_light()

/atom/proc/demir_tag_light()
	for(var/image/underlay in underlays)
		underlay.demir_tag_light()
	for(var/image/overlay in overlays)
		overlay.demir_tag_light()

/datum/demir/goonstation/bake(atom/target)
	if(istype(target, /obj/table))
		var/obj/table/table = target
		if(table.auto && table.materialless_icon_state() == "0")
			table.set_up()
			target.demir_tag_light()
			return
	target.UpdateIcon()
	target.demir_bake_extras()
	target.demir_tag_light()

// RobustLight2 precomputes a point light's radius from its brightness and
// height. Use that native calculation, then export the attenuation parameters
// consumed by the editor's static lighting pass.
/datum/light/proc/demir_export(atom/target)
	target.demir_light_range = radius
	target.demir_light_power = brightness
	target.demir_light_color = rgb(r * 255, g * 255, b * 255)
	target.demir_light_height = height ** 2
	target.demir_light_quadratic = brightness * RL_Atten_Quadratic
	target.demir_light_constant = RL_Atten_Constant

// A preview light has no world position. The native constructor only copies
// its coordinates and registers a positioned light with its turf.
/datum/light/point/New()
	return

/atom/proc/demir_apply_point_light(brightness, red, green, blue, height = 1)
	if(!brightness)
		return
	var/datum/light/point/preview_light = new
	preview_light.set_brightness(brightness)
	preview_light.set_color(red, green, blue)
	preview_light.set_height(height)
	preview_light.demir_export(src)

// RobustLight2 never starts in the editor, so an attached light only keeps its settings and
// whether it was enabled.
/atom/proc/demir_attach_light(brightness, red = 1, green = 1, blue = 1, height = 1, enabled = TRUE)
	var/datum/light/point/light = new
	light.set_brightness(brightness)
	light.set_color(red, green, blue)
	light.set_height(height)
	light.attach(src)
	if(enabled)
		light.enable()
	return light

/atom/proc/demir_prepare_lights()
	return

/obj/machinery/camera/demir_prepare_lights()
	if(has_light)
		light = demir_attach_light(0.3, 209 / 255, 27 / 255, 6 / 255)

// New() reads the switch position from the area half a second in, and update_icon() colours it.
/obj/machinery/light_switch/demir_prepare_lights()
	area = otherarea ? locate(text2path("/area/[otherarea]")) : get_area(src)
	on = area ? area.lightswitch : on
	light = demir_attach_light(0.3, on ? 0.5 : 1, on ? 1 : 0.5, 0.5, enabled = powered())

/obj/machinery/computer/demir_prepare_lights()
	base_icon_state = initial(icon_state)
	light = demir_attach_light(0.4, light_r, light_g, light_b, enabled = !(status & BROKEN) && powered())

/obj/machinery/computer3/demir_prepare_lights()
	base_icon_state = icon_state
	light = demir_attach_light(0.4, enabled = !(status & BROKEN) && powered())

/obj/machinery/vending/demir_prepare_lights()
	light = demir_attach_light(0.6, light_r, light_g, light_b, 1.5, !fallen && !(status & BROKEN) && powered())

/obj/machinery/r_door_control/demir_prepare_lights()
	light = demir_attach_light(0.6, 0.9, 0.5, 0.5, 1.25)

/obj/machinery/genetics_booth/demir_prepare_lights()
	light = demir_attach_light(0.6, light_r, light_g, light_b, 1.5, powered())

/obj/machinery/clothingbooth/demir_prepare_lights()
	ambient_light = demir_attach_light(0.6, height = 1.5, enabled = powered())

/obj/machinery/chemicompiler_stationary/demir_prepare_lights()
	light = demir_attach_light(0.4, enabled = !(status & BROKEN) && powered())

/obj/machinery/disposal/chemlink/demir_prepare_lights()
	light = demir_attach_light(0.4, 0.7, 1, 0.7, enabled = powered())

/obj/machinery/atmospherics/unary/cryo_cell/demir_prepare_lights()
	light = demir_attach_light(0.6, 0, 0.8, 0.5, 1.5, on)

/obj/machinery/hydro_growlamp/demir_prepare_lights()
	light = demir_attach_light(1, 0.7, 0.2, 1, enabled = active)

/obj/submachine/GTM/demir_prepare_lights()
	light = demir_attach_light(0.4)

/obj/decal/xmas_lights/demir_prepare_lights()
	light = demir_attach_light(0.3, 0.2, 0.6, 0.9)

/obj/burning_barrel/demir_prepare_lights()
	light = demir_attach_light(1, 0.5, 0.3, 0, enabled = on)

/obj/decal/glow/demir_prepare_lights()
	light = demir_attach_light(brightness / 5, color_r, color_g, color_b)

/obj/decoration/candles/demir_prepare_lights()
	light = demir_attach_light(brightness, col_r, col_g, col_b, enabled = lit == 1)

/obj/decoration/regallamp/demir_prepare_lights()
	light = demir_attach_light(brightness, col_r, col_g, col_b, enabled = lit == 1)

/atom/proc/demir_apply_light()
	for(var/datum/light/light in RL_Attached)
		if(light.enabled)
			light.demir_export(src)

// Areas and turfs can opt out of RobustLight2's darkness independently of
// point lights. Ambient area colors are already stored in BYOND color form.
/area/demir_apply_light()
	demir_fullbright = force_fullbright
	if(ambient_light)
		demir_ambient_color = ambient_light
		demir_ambient_power = 1

/turf/demir_apply_light()
	demir_fullbright = fullbright
	..()

// The normal world startup calls power_change() before RobustLight2 starts.
// Reproduce only the state decisions that select mapped, statically powered
// fixtures; their game machinery and power networks remain stopped.
/obj/machinery/light/proc/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && light_area.lightswitch && light_area.power_light

/obj/machinery/light/emergency/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && (!light_area.power_light || shipAlertState == SHIP_ALERT_BAD)

/obj/machinery/light/traffic_light/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && on && light_area.power_light

/obj/machinery/light/lamp/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && switchon && light_area.power_light

// These subtypes normally mark themselves broken in New(), which the preview
// deliberately does not execute for mapped atoms.
/obj/machinery/light/small/broken/demir_is_on()
	return FALSE

/obj/machinery/light/small/sticky/broken/demir_is_on()
	return FALSE

/obj/machinery/light/small/floor/broken/demir_is_on()
	return FALSE

/obj/machinery/light/incandescent/broken/demir_is_on()
	return FALSE

/obj/machinery/light/proc/demir_fixture_color()
	return list(initial(src.light_type.color_r), initial(src.light_type.color_g), initial(src.light_type.color_b))

// These fixtures replace their light datum's color in New(). Keep those
// declared constructor effects without running the rest of machinery setup.
/obj/machinery/light/lamp/green/demir_fixture_color()
	return list(0.45, 0.85, 0.25)

/obj/machinery/light/flock/demir_fixture_color()
	return list(0.45, 0.75, 0.675)

/obj/machinery/light/demir_apply_light()
	if(!demir_is_on())
		return
	var/list/source_color = demir_fixture_color()
	demir_apply_point_light(brightness, source_color[1], source_color[2], source_color[3], 2.4)

// Map lights are direct RobustLight2 sources. Their native constructor scales
// the mapped brightness before enabling the point light.
/obj/map/light/demir_apply_light()
	demir_apply_point_light(brightness / 5, color_r, color_g, color_b)

/datum/demir/goonstation/light(atom/target)
	target.demir_apply_light()

#endif
