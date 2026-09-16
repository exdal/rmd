// A template, not something rmd loads on its own. Copy this file into a Goonstation checkout and
// `#include` it from `goonstation.dme`; the guard keeps BYOND compiling it to nothing.
//
// Goonstation derives its icons in `UpdateIcon()` and keeps its per-type data in a typeinfo datum,
// so the profile resolves that datum per atom and lets the codebase pick its own icon state.
#ifdef __DEMIR_BAKE__

/proc/demir_bake(atom/target)
	if(istype(target, /obj/table))
		var/obj/table/table = target
		if(table.auto && table.materialless_icon_state() == "0")
			table.set_up()
			return
	target.UpdateIcon()

/proc/demir_prepare(atom/target)
	target.get_typeinfo()

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

/atom/proc/demir_apply_light()
	return

// Areas and turfs can opt out of RobustLight2's darkness independently of
// point lights. Ambient area colors are already stored in BYOND color form.
/area/demir_apply_light()
	demir_fullbright = force_fullbright
	if(ambient_light)
		demir_ambient_color = ambient_light
		demir_ambient_power = 1

/turf/demir_apply_light()
	demir_fullbright = fullbright

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

/proc/demir_light(atom/target)
	target.demir_apply_light()

#endif
