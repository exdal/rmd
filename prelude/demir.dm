#define __DEMIR__

#ifndef __DEMIR_BAKE__
#define __DEMIR_COMPAT__
#endif
#ifdef __DEMIR_COMPAT__
// mapping tools set this, and codebases gate map editor icon states on it
#define FASTDMM  // we do a little bit of lying
#define SPACEMAN_DMM
#define SpacemanDMM_unlint(X) X
#define SpacemanDMM_debug(X...) X
#endif

///
/// LIGHTING AND APPEARANCE SCHEMA
///

/atom
	var/demir_light_range = 0
	/// Radius of full brightness inside `demir_light_range`.
	var/demir_light_inner_range = 0
	var/demir_light_power = 0
	var/demir_light_color = null
	var/demir_light_angle = 360
	var/demir_light_dir = 0
	/// Additional light-source origin offset in tile units.
	var/demir_light_offset_x = 0
	var/demir_light_offset_y = 0
	var/demir_light_height = 1
	/// Falloff exponent.
	var/demir_light_curve = 1
	/// Combine with other peak sources by taking the strongest instead of summing.
	var/demir_light_peak = 0
	/// Only contribute when this cell touches one that is not fullbright.
	var/demir_light_edge_only = 0
	/// Use `demir_light_quadratic / distance ** 2` when nonzero.
	var/demir_light_quadratic = 0
	/// Constant added to the quadratic term.
	var/demir_light_constant = 0
	/// A value of -1 follows `opacity`.
	var/demir_blocks_light = -1
	var/demir_ambient_color = null
	/// Ambient power in the range 0..1.
	var/demir_ambient_power = 0
	/// Suppress corner lighting for this cell.
	var/demir_fullbright = 0
	/// Use this appearance as an emissive mask without drawing it into the scene color.
	var/demir_emissive = 0
	/// Clear emissive pixels behind this appearance without drawing it.
	var/demir_emissive_blocker = 0
	/// Add (positive) or subtract (negative) this appearance from the cheap overlay lightmap.
	var/demir_overlay_light = 0

/image
	var/demir_emissive = 0
	var/demir_emissive_blocker = 0
	var/demir_overlay_light = 0

///
/// BAKING HOOKS
///

#define DEMIR_CONNECTION_SOURCE 1
#define DEMIR_CONNECTION_TARGET 2

#define DEMIR_HIGHLIGHT_SELECTED 1
#define DEMIR_HIGHLIGHT_HOVERED 2
#define DEMIR_HIGHLIGHT_ALWAYS 4

#define USE_PERSPECTIVE_EDITOR_WALLS

// A codebase teaches the editor how to bake its maps with one subtype of this. The editor builds that subtype once
// per bake and calls the hooks below on the instance, so what a profile wants to remember is a var
// on it rather than a global. A codebase may carry several profiles as long as one is more derived
// than the rest, the editor bakes with the subtype nothing else inherits from.
//
//   /datum/demir/tgstation
//     var/smooth = TRUE
//
//     New()
//       ..()
//       demir_define_group(GROUP_CABLES, /obj/structure/cable)
//
//     bake(atom/target)
//       target.icon_state = smooth ? "wall" : "plain"
//
// New() sets the profile up. It runs once, before every other hook, and is the only place
// demir_define_group() is callable. A proc that is not itself a hook reaches the same instance
// through demir_profile().
/datum/demir
	proc/bake(atom/target)

	proc/prepare(atom/target)

	proc/light(atom/target)

	// Return an associative list of opaque text channel keys to DEMIR_CONNECTION_* role bitmasks.
	proc/connections(atom/target)

	// Return a list of tile regions the editor should shade for this atom. Each entry is an
	// associative list, unknown keys are ignored.
	//
	//     "x", "y", "width", "height"  a rectangle, offset in tiles from this atom's own tile
	//     "tiles"                      list(list(x, y), ...) offsets, instead of a rectangle
	//     "color"                      any hex color, default orange
	//     "fill"                       wash opacity from 0 to 1, default 0.12
	//     "outline"                    draw the marching border, default 1
	//     "when"                       DEMIR_HIGHLIGHT_* bitmask, default DEMIR_HIGHLIGHT_SELECTED
	//     "label"                      text drawn above the region
	proc/highlights(atom/target)

	// Called once per editor frame with the selected atom, or null. The imgui_* procs below only
	// run inside it.
	//
	// The editor, not the profile, remembers what the viewer set: a widget takes the profile's
	// value as its starting one and then answers with the edited value every frame after. A button
	// answers true on the frame after it is pressed.
	//
	// Writes roll back on every frame but the one that first carries a click or an edit and writes
	// something. That is what lets the profile's own vars hold panel state without an idle profile
	// growing the heap:
	//
	//   /datum/demir/example
	//     var/smooth = TRUE
	//
	//     ui(atom/target)
	//       smooth = imgui_checkbox("Smooth walls", smooth)
	//
	// A frame that keeps its writes re-derives appearances, highlights and lighting, so the other
	// hooks see the new state. New() does not run again, so the state survives.
	proc/ui(atom/target)

// The profile editor built for this bake, so a proc that is not itself a hook can read what the panel
// wrote. Null outside a bake.
/proc/demir_profile()
	set __demir_intrin = 720

///
/// REBAKING
///

// Put `type` and everything under it in the group `group`, a bit demir_rebake() can then name to
// re-derive only those placements. Only callable from a profile's New(). The meaning of each bit is
// the profile's own, the editor only matches them. Call it once per type, in any order, and a type may join
// several groups.
/proc/demir_define_group(group, type)
	set __demir_intrin = 719

// What a demir_rebake() call re-derives. Combinable.
#define DEMIR_BAKE_APPEARANCE 1
#define DEMIR_BAKE_LIGHT 2
#define DEMIR_BAKE_HIGHLIGHT 4

// Ask for part of the map to be re-derived, because something ui() just wrote changes what
// another hook answers. Only callable from ui(), and only a frame that keeps its writes acts
// on it.
//
// `groups` selects the placements to redo by the groups demir_define_group() put their type in, so
// an option that only drives one kind of atom costs one kind of atom. Zero, or omitted, means every
// placement. Several calls in a frame add up.
//
// A frame that asks for nothing re-derives nothing, so a profile that writes state without calling
// this draws a panel whose switches do not take effect.
/proc/demir_rebake(kinds, groups = 0)
	set __demir_intrin = 718
