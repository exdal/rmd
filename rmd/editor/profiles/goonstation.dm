// rmd embeds this profile for the Forced bundled profile setting. To keep it in a Goonstation
// checkout instead, copy it there and `#include` it from `goonstation.dme` after `// END_INCLUDE`.
// BYOND skips it because of the guard below.
//
// Goonstation picks icons in UpdateIcon() and keeps per-type data in a typeinfo datum, so prepare
// resolves the typeinfo of each atom. Mapped atoms never run New(), so the state UpdateIcon()
// reads and the lights New() attaches are set up here.
#ifdef __DEMIR_BAKE__

#ifndef USE_PERSPECTIVE_EDITOR_WALLS
#define USE_PERSPECTIVE_EDITOR_WALLS
#endif

#define DEMIR_GROUP_POWER (1<<0)

/atom/proc/demir_prepare_state()
	return

/turf/simulated/wall/auto/asteroid/demir_prepare_state()
	LAZYLISTINIT(topoverlaycache)
	space_overlays = list()
	topnumber = pick(1, 2, 3)
	orenumber = pick(1, 2, 3)
	color = stone_color

/obj/machinery/firealarm/demir_prepare_state()
	alarm_base_overlay = image(icon, src, "fireoff")
	alarm_overlay = image(icon, src, "fireoff")
	alarm_overlay.plane = PLANE_LIGHTING
	alarm_overlay.blend_mode = BLEND_ADD
	alarm_overlay.layer = LIGHTING_LAYER_BASE
	alarm_overlay.alpha = 80

/obj/machinery/portable_atmospherics/canister/demir_prepare_state()
	atmos_dmi = image('icons/obj/atmospherics/atmos.dmi')
	bomb_dmi = image('icons/obj/canisterbomb.dmi')

/obj/disposalpipe/demir_prepare_state()
	base_icon_state = icon_state

/obj/cable/demir_prepare_state()
	var/separator = findtext(icon_state, "-")
	if(separator)
		d1 = text2num(copytext(icon_state, 1, separator))
		d2 = text2num(copytext(icon_state, separator + 1))

/obj/cable/auto/demir_prepare_state()
	d1 = 0
	d2 = 0

/obj/item/demir_prepare_state()
	if(inventory_counter_enabled)
		create_inventory_counter()
		if(amount != 1)
			inventory_counter.update_number(amount)

/obj/item/reagent_containers/demir_prepare_state()
	..()
	ensure_reagent_holder()
	create_initial_reagents()

/obj/item/reagent_containers/food/snacks/demir_prepare_state()
	if(!uneaten_bites_left)
		uneaten_bites_left = initial(bites_left)
	..()

/obj/item/reagent_containers/food/snacks/ingredient/egg/chicken/demir_prepare_state()
	..()
	if(egg_props_path)
		chicken_egg_props = new egg_props_path(src)

/obj/machinery/nuclear_reactor/demir_prepare_state()
	input = new /obj/machinery/atmospherics/unary/node
	output = new /obj/machinery/atmospherics/unary/node

/obj/item/ammo/bullets/tranq_darts/demir_prepare_state()
	..()
	demir_cache_reagent(ammo_type?.reagent_payload)

/obj/item/gun/reagent/demir_prepare_state()
	..()
	create_reagents(capacity)

/obj/item/gun/energy/tasersmg/demir_prepare_state()
	..()
	set_current_projectile(new /datum/projectile/energy_bolt/smgburst)
	projectiles = list(current_projectile, new /datum/projectile/energy_bolt/smgauto)

/obj/item/sword/demir_prepare_state()
	..()
	AddComponent(/datum/component/loctargeting/simple_light, 255, 255, 255, 150)

/obj/reagent_dispensers/demir_prepare_state()
	create_reagents(capacity)

/obj/machinery/clonegrinder/demir_prepare_state()
	create_reagents(100)

/obj/machinery/fluid_canister/demir_prepare_state()
	create_reagents(bladder)

/obj/item/material_piece/demir_prepare_state()
	..()
	if(!material && default_material)
		setMaterial(getMaterial(default_material))

/obj/item/gun/kinetic/demir_prepare_state()
	..()
	if(!ammo && default_magazine)
		ammo = new default_magazine

/obj/machinery/portable_atmospherics/pump/demir_prepare_state()
	tank_hatch = image('icons/obj/atmospherics/atmos.dmi', "")

/obj/machinery/networked/test_apparatus/heater/demir_prepare_state()
	heat_overlay = image('icons/obj/networked.dmi', "")

/obj/machinery/vending/player/chemicals/demir_prepare_state()
	fill_image = image(icon, "medchem-0", -1)
	fill_image.plane = PLANE_ABOVE_LIGHTING

/obj/storage/crate/loot/demir_prepare_state()
	light = image('icons/obj/storage/crate.dmi', "lootcratelocklight")

/obj/machinery/interdictor/demir_prepare_state()
	intcap = new /obj/item/cell/supercell/charged(src)

/obj/machinery/gravity_tether/demir_prepare_state()
	cell = new cell_type_path(src)
	last_charge_amount = cell.charge
	MA = mutable_appearance(icon, icon_state)
	ma_graviton = mutable_appearance(icon, "graviton-idle")
	ma_door = mutable_appearance(icon, "door-closed")
	ma_cell = mutable_appearance('icons/obj/power.dmi', "apc-hpcell")
	ma_cell.pixel_y = -4
	ma_cell.dir = SOUTH
	ma_cell_charge = mutable_appearance('icons/obj/power.dmi', "cell02")
	ma_cell_charge.pixel_y = 1
	ma_cell_charge.pixel_x = 1
	ma_cell_charge.dir = SOUTH
	ma_wires = mutable_appearance(icon, "wires-intact")
	ma_tamper = mutable_appearance(icon, "tamper-secure")
	ma_bat = mutable_appearance(icon, "battery-full")
	ma_bat_charge = mutable_appearance(icon, "power-charging")
	ma_status = mutable_appearance(icon, "status-working")
	ma_graph = mutable_appearance(icon, "graph-good")
	ma_screen = mutable_appearance(icon, "screen-locked")
	ma_intensity = mutable_appearance(icon, "level-2")
	ma_dials = mutable_appearance(icon, "dials-regular")
	update_ma_bat()
	update_ma_tamper()
	update_ma_cell()
	update_ma_screen()

/obj/machinery/power/lgenerator/demir_prepare_state()
	spin_sprite = new /image(icon, "ggen-spin")
	tank_sprite = new /image(icon, "ggen-tank")

/obj/item/tripod_bulb/demir_bake_icon()
	return

/obj/item/material_piece/demir_bake_icon()
	if(material)
		..()

/obj/item/assembly/demir_bake_icon()
	return

/datum/demir/goonstation
	default = TRUE
	var/power_all = TRUE

	New()
		..()
		demir_define_group(DEMIR_GROUP_POWER, /obj/machinery)

// ui() writes are rolled back unless the viewer changed something that frame, so panel state
// lives in the profile's vars.
/datum/demir/goonstation/ui(atom/target)
	if(!imgui_begin("Goonstation"))
		imgui_end()
		return

	imgui_separator("Power")
	var/is_powered_all = imgui_checkbox("All areas powered", src.power_all)
	if(is_powered_all != src.power_all)
		src.power_all = is_powered_all
		demir_rebake(DEMIR_BAKE_LIGHT, DEMIR_GROUP_POWER)
		demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_POWER)

	imgui_end()

// When unchecked, only areas that need no power are powered.
/proc/demir_area_powered(area/A, chan)
	var/datum/demir/goonstation/profile = demir_profile()
	if(profile.power_all)
		return TRUE

	return A && !A.requires_power

/obj/machinery/powered(chan = EQUIP)
	if(istype(loc, /obj/item/electronics/frame))
		return FALSE

	var/area/A = get_area(src)
	if(!A)
		return FALSE

	return demir_area_powered(A, chan)

/proc/getMaterial(mat)
	if(!istext(mat))
		return null
	if(!material_cache)
		material_cache = list()
	if(!(mat in material_cache))
		for(var/datum/material/type as anything in concrete_typesof(/datum/material))
			if(initial(type.cached) && initial(type.mat_id) == mat)
				material_cache[mat] = new type()
				break
	return material_cache[mat]

/proc/demir_cache_reagent(reagent)
	if(!istext(reagent) || (reagent in reagents_cache))
		return
	for(var/datum/reagent/type as anything in concrete_typesof(/datum/reagent))
		if(initial(type.id) == reagent)
			reagents_cache[reagent] = new type()
			return

/datum/reagents/add_reagent(reagent, amount, sdata, temp_new = T20C, donotreact = 0, donotupdate = 0, chemical_reaction = FALSE, chem_reaction_priority = 1)
	demir_cache_reagent(reagent)
	return ..()

/datum/bioluminescent_algae/get_color(atom/A, force = FALSE)
	return null

/proc/logTheThing(type, source, text, diaryType)
	return

/proc/build_chem_structure()
	return

/datum/demir/goonstation/prepare(atom/target)
	target.get_typeinfo()
	target.demir_prepare_state()
	target.demir_prepare_lights()

/atom/proc/demir_bake_extras()
	return

/turf/simulated/wall/auto/asteroid/demir_bake_extras()
	space_overlays()

/turf/simulated/wall/auto/asteroid/top_overlays()
	return

/turf/simulated/wall/auto/asteroid/ore_overlays()
	if(ore)
		var/image/ore_overlay = mutable_appearance('icons/turf/walls/asteroid.dmi', "[ore.name][orenumber]")
		ore_overlay.layer = ASTEROID_ORE_OVERLAY_LAYER
		AddOverlays(ore_overlay, "ast_ore")

/turf/space/demir_bake_extras()
	if(!length(underlays))
		return
	for(var/direction in alldirs)
		var/turf/neighbor = get_step(src, direction)
		if(neighbor && !neighbor.fullbright)
			return
	underlays = list()

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

// The /obj/overlay/simple_light that add_simple_light() puts in vis_contents, as an image.
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

// Images on the lighting plane add light, and subtract blends remove it.
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

/atom/proc/demir_bake_icon()
	UpdateIcon()

// Runtime areas hide their mapping icon in update_icon().
/area/demir_bake_icon()
	return

// Conduits and other special mapped states are not endpoint pairs.
/obj/cable/demir_bake_icon()
	if(findtext(icon_state, "-"))
		UpdateIcon()

// check() and replace() lay real cables, build powernets and delete the spawner. check() also
// scans only half the directions and relies on its neighbors to set the rest. This works out the
// same cables from this spawner alone and draws them.
/obj/cable/auto/demir_bake_icon()
	var/list/connected = list()
	for(var/direction in list(NORTH, SOUTH, EAST, WEST, NORTHEAST, NORTHWEST, SOUTHEAST, SOUTHWEST))
		var/turf/neighbor = get_step(src, direction)
		if(!neighbor)
			continue
		var/found = FALSE
		for(var/obj/cable/auto/spawner in neighbor)
			if(spawner.cable_type == cable_type || spawner.color == color)
				found = TRUE
				break
		if(found)
			connected += direction

	// The spawner drops a diagonal when a cardinal next to it is connected.
	if((NORTHEAST in connected) && ((NORTH in connected) || (EAST in connected)))
		connected -= NORTHEAST
	if((NORTHWEST in connected) && ((NORTH in connected) || (WEST in connected)))
		connected -= NORTHWEST
	if((SOUTHEAST in connected) && ((SOUTH in connected) || (EAST in connected)))
		connected -= SOUTHEAST
	if((SOUTHWEST in connected) && ((SOUTH in connected) || (WEST in connected)))
		connected -= SOUTHWEST

	// Mapped cables are added after diagonals are dropped, because the spawner always links to them.
	for(var/direction in list(NORTH, SOUTH, EAST, WEST, NORTHEAST, NORTHWEST, SOUTHEAST, SOUTHWEST))
		if(direction in connected)
			continue
		var/turf/neighbor = get_step(src, direction)
		if(!neighbor)
			continue
		for(var/obj/cable/cable in neighbor)
			if(istype(cable, /obj/cable/auto) || (cable.type != cable_type && cable.color != color))
				continue
			if(!istype(cable, cable_type) && !istype(cable_type, cable))
				continue
			var/back = turn(direction, 180)
			if(cable.d1 == back || cable.d2 == back)
				connected += direction
				break

	var/list/directions = list()
	for(var/direction in list(NORTH, NORTHEAST, EAST, SOUTHEAST, SOUTH, SOUTHWEST, WEST, NORTHWEST))
		if(direction in connected)
			directions += direction

	var/center = override_centre_connection
	for(var/obj/machinery/power/machine in loc)
		if(istype(machine, /obj/machinery/power/terminal) || istype(machine, /obj/machinery/power/smes))
			center = TRUE
			break

	var/suffix = initial(cable_type.iconmod)
	if(isnull(suffix) && istype(src, /obj/cable/auto/reinforced))
		suffix = "-thick"
	var/list/states = list()
	if(!length(directions))
		states += "0-1[suffix]"
	else if(center || length(directions) == 1)
		for(var/direction in directions)
			states += "0-[direction][suffix]"
	else if(length(directions) == 2)
		states += "[min(directions[1], directions[2])]-[max(directions[1], directions[2])][suffix]"
	else
		for(var/i in 1 to length(directions) - 1)
			states += "[min(directions[i], directions[i + 1])]-[max(directions[i], directions[i + 1])][suffix]"
		states += "[min(directions[1], directions[length(directions)])]-[max(directions[1], directions[length(directions)])][suffix]"

	if(length(states) == 1)
		icon_state = states[1]
		return
	icon_state = null
	overlays = list()
	for(var/state in states)
		overlays += mutable_appearance(icon, state)

/atom/proc/demir_bake_appearance()
	demir_bake_icon()
	demir_bake_extras()

/obj/table/demir_bake_appearance()
	if(auto && materialless_icon_state() == "0")
		set_up()
		return
	return ..()

/datum/demir/goonstation/bake(atom/target)
	target.demir_bake_appearance()
	target.demir_tag_light()

/atom/proc/demir_highlights()
	return null

/obj/machinery/camera/demir_highlights()
	var/turf/origin = get_turf(src)
	if(!origin)
		return null

	var/list/visible = list()
	if(camera_status)
		for(var/turf/T in view(CAM_RANGE, origin))
			visible[T] = TRUE

	var/list/seen = list()
	var/list/blind = list()
	for(var/turf/T as anything in block(origin.x - CAM_RANGE, origin.y - CAM_RANGE, origin.z, origin.x + CAM_RANGE, origin.y + CAM_RANGE))
		if(visible[T])
			seen += list(list(T.x - origin.x, T.y - origin.y))
		else
			blind += list(list(T.x - origin.x, T.y - origin.y))

	var/when = DEMIR_HIGHLIGHT_SELECTED | DEMIR_HIGHLIGHT_HOVERED
	return list(
		list("tiles" = seen, "color" = "#40c040", "fill" = 0.25, "when" = when),
		list("tiles" = blind, "color" = "#e04040", "fill" = 0.25, "when" = when),
		list("x" = 0, "y" = 0, "width" = 1, "height" = 1, "fill" = 0, "outline" = 0, "when" = when, "label" = network),
	)

/datum/demir/goonstation/highlights(atom/target)
	return target.demir_highlights()

// precalc() computes the radius from brightness and height. Export it with the attenuation values
// the editor's lighting pass uses.
/datum/light/proc/demir_export(atom/target)
	target.demir_light_range = radius
	target.demir_light_power = brightness
	target.demir_light_color = rgb(r * 255, g * 255, b * 255)
	target.demir_light_height = height ** 2
	target.demir_light_quadratic = brightness * RL_Atten_Quadratic
	target.demir_light_constant = RL_Atten_Constant

// Preview lights have no position. The real New() only stores coordinates and registers the light
// with its turf.
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
// whether it is enabled.
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

// New() reads the switch position from the area half a second in, and update_icon() colors it.
/obj/machinery/light_switch/demir_prepare_lights()
	area = otherarea ? locate(text2path("/area/[otherarea]")) : get_area(src)
	on = area ? area.lightswitch : on
	light = demir_attach_light(0.3, on ? 0.5 : 1, on ? 1 : 0.5, 0.5, enabled = FALSE)

/obj/machinery/computer/demir_prepare_lights()
	base_icon_state = initial(icon_state)
	light = demir_attach_light(0.4, light_r, light_g, light_b)

/obj/machinery/computer3/demir_prepare_lights()
	base_icon_state = icon_state
	light = demir_attach_light(0.4)

/obj/machinery/vending/demir_prepare_lights()
	light = demir_attach_light(0.6, light_r, light_g, light_b, 1.5)

/obj/machinery/r_door_control/demir_prepare_lights()
	light = demir_attach_light(0.6, 0.9, 0.5, 0.5, 1.25)

/obj/machinery/genetics_booth/demir_prepare_lights()
	light = demir_attach_light(0.6, light_r, light_g, light_b, 1.5)

/obj/machinery/clothingbooth/demir_prepare_lights()
	ambient_light = demir_attach_light(0.6, height = 1.5)

/obj/machinery/chemicompiler_stationary/demir_prepare_lights()
	light = demir_attach_light(0.4)

/obj/machinery/disposal/chemlink/demir_prepare_lights()
	light = demir_attach_light(0.4, 0.7, 1, 0.7)

/obj/machinery/atmospherics/unary/cryo_cell/demir_prepare_lights()
	light = demir_attach_light(0.6, 0, 0.8, 0.5, 1.5, on)

/obj/machinery/hydro_growlamp/demir_prepare_lights()
	light = demir_attach_light(1, 0.7, 0.2, 1, enabled = active)

/obj/submachine/GTM/demir_prepare_lights()
	light = demir_attach_light(0.4)

// update_icon() switches these on and off.
/obj/machinery/networked/secdetector/demir_prepare_lights()
	light = demir_attach_light(1, 0.2, 0.65, 0.2, enabled = FALSE)

/obj/machinery/power/lgenerator/demir_prepare_lights()
	light = demir_attach_light(0.8, enabled = FALSE)

/obj/machinery/power/catalytic_generator/demir_prepare_lights()
	light = demir_attach_light(0.3, enabled = FALSE)

/obj/machinery/gravity_tether/demir_prepare_lights()
	light = demir_attach_light(0.8, height = 1.5, enabled = FALSE)

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

/atom/proc/demir_set_light_enabled(datum/light/light, is_enabled)
	if(is_enabled)
		light.enable()
	else
		light.disable()

// A rebake keeps the previous run's heap, so every power-dependent decision is made here.
/obj/machinery/demir_apply_light()
	if(powered())
		status &= ~NOPOWER
	else
		status |= NOPOWER

	return ..()

/obj/machinery/light_switch/demir_apply_light()
	demir_set_light_enabled(light, powered())
	return ..()

/obj/machinery/computer/demir_apply_light()
	demir_set_light_enabled(light, !(status & BROKEN) && powered())
	return ..()

/obj/machinery/computer3/demir_apply_light()
	demir_set_light_enabled(light, !(status & BROKEN) && powered())
	return ..()

/obj/machinery/vending/demir_apply_light()
	demir_set_light_enabled(light, !fallen && !(status & BROKEN) && powered())
	return ..()

/obj/machinery/genetics_booth/demir_apply_light()
	demir_set_light_enabled(light, powered())
	return ..()

/obj/machinery/clothingbooth/demir_apply_light()
	demir_set_light_enabled(ambient_light, powered())
	return ..()

/obj/machinery/chemicompiler_stationary/demir_apply_light()
	demir_set_light_enabled(light, !(status & BROKEN) && powered())
	return ..()

/obj/machinery/disposal/chemlink/demir_apply_light()
	demir_set_light_enabled(light, powered())
	return ..()

// Areas and turfs can be fullbright whatever their point lights do. Area ambient colors are
// already BYOND color strings.
/area/demir_apply_light()
	demir_fullbright = force_fullbright
	if(ambient_light)
		demir_ambient_color = ambient_light
		demir_ambient_power = 1

/turf/demir_apply_light()
	demir_fullbright = fullbright
	..()

// World startup calls power_change() before RobustLight2 starts. These only decide which fixtures
// are lit. Machinery and power networks stay stopped.
/obj/machinery/light
	var/demir_lit = FALSE

/obj/machinery/light/proc/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && light_area.lightswitch && demir_area_powered(light_area, LIGHT)

/obj/machinery/light/emergency/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && (!demir_area_powered(light_area, LIGHT) || shipAlertState == SHIP_ALERT_BAD)

/obj/machinery/light/traffic_light/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && on && demir_area_powered(light_area, LIGHT)

/obj/machinery/light/lamp/demir_is_on()
	var/area/light_area = get_area(src)
	return light_area && switchon && demir_area_powered(light_area, LIGHT)

// These subtypes set LIGHT_BROKEN in New(), which mapped atoms never run.
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

// These fixtures change their light color in New().
/obj/machinery/light/lamp/green/demir_fixture_color()
	return list(0.45, 0.85, 0.25)

/obj/machinery/light/flock/demir_fixture_color()
	return list(0.45, 0.75, 0.675)

/obj/machinery/light/demir_apply_light()
	..()
	demir_lit = demir_is_on()
	if(!demir_lit)
		return

	var/list/source_color = demir_fixture_color()
	demir_apply_point_light(brightness, source_color[1], source_color[2], source_color[3], 2.4)

// Mapped fixtures keep their icon_state because New() never inserts a lamp for update_icon_state().
/obj/machinery/light/demir_bake_icon()
	..()
	var/state_on = "[base_state]1"
	var/state_off = "[base_state]0"
	if(icon_state == state_on || icon_state == state_off)
		icon_state = demir_lit ? state_on : state_off

/obj/machinery/light/incandescent/demir_bake_icon()
	autoposition(instant = TRUE)
	..()

// Map lights are plain RobustLight2 sources. New() scales the mapped brightness before enabling
// them.
/obj/map/light/demir_apply_light()
	demir_apply_point_light(brightness / 5, color_r, color_g, color_b)

/datum/demir/goonstation/light(atom/target)
	target.demir_apply_light()

#undef DEMIR_GROUP_POWER

#endif
