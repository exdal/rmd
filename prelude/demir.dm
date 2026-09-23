#define __DEMIR__

#ifndef __DEMIR_BAKE__
#define __DEMIR_COMPAT__
#endif
#ifdef __DEMIR_COMPAT__
// Supported codebases use these names to select mapping-only code.
#define FASTDMM
#define SPACEMAN_DMM
#define SpacemanDMM_unlint(X) X
#define SpacemanDMM_debug(X...) X
#endif

// Lighting and appearance fields

/atom
	var/demir_light_range = 0
	/// Set the radius of full brightness within `demir_light_range`.
	var/demir_light_inner_range = 0
	var/demir_light_power = 0
	var/demir_light_color = null
	var/demir_light_angle = 360
	var/demir_light_dir = 0
	/// Offset the light source on the x axis in tile units.
	var/demir_light_offset_x = 0
	var/demir_light_offset_y = 0
	var/demir_light_height = 1
	/// Set the light falloff exponent.
	var/demir_light_curve = 1
	/// Use the strongest peak source at each sample.
	var/demir_light_peak = 0
	/// Emit light only along the edge of a fullbright region.
	var/demir_light_edge_only = 0
	/// Use `demir_light_quadratic / distance ** 2` when this value is nonzero.
	var/demir_light_quadratic = 0
	/// Add this constant to the quadratic term.
	var/demir_light_constant = 0
	/// Use `opacity` when this value is -1.
	var/demir_blocks_light = -1
	var/demir_ambient_color = null
	/// Set ambient power from 0 to 1.
	var/demir_ambient_power = 0
	/// Disable corner lighting for this cell.
	var/demir_fullbright = 0
	/// Use this appearance as an emissive mask and omit it from the scene color.
	var/demir_emissive = 0
	/// Clear emissive pixels behind this appearance and omit the appearance from the scene.
	var/demir_emissive_blocker = 0
	/// Add this appearance for positive values. Subtract it for negative values.
	var/demir_overlay_light = 0

/image
	var/demir_emissive = 0
	var/demir_emissive_blocker = 0
	var/demir_overlay_light = 0

// Appearance profile

#define DEMIR_CONNECTION_SOURCE 1
#define DEMIR_CONNECTION_TARGET 2

#define DEMIR_HIGHLIGHT_SELECTED 1
#define DEMIR_HIGHLIGHT_HOVERED 2
#define DEMIR_HIGHLIGHT_ALWAYS 4

// Define a subtype to teach the editor how to bake a codebase. The bake runtime creates one profile
// instance and calls these hooks on it. Store persistent profile state in its variables.
//
// Profiles can inherit from other profiles. Every subtype is selectable. Exactly one subtype must
// directly set default to a true value; descendants do not inherit that selection marker.
//
//   /datum/demir/tgstation
//     default = TRUE
//     var/smooth = TRUE
//
//     New()
//       ..()
//       demir_define_group(GROUP_CABLES, /obj/structure/cable)
//
//     bake(atom/target)
//       target.icon_state = smooth ? "wall" : "plain"
//
// New() runs once before all hooks. Use it for setup and demir_define_group() calls. An ordinary
// procedure can get the same profile instance through demir_profile().
/datum/demir
	/// Select this profile when the user has not chosen another one. Exactly one profile subtype
	/// must directly set this to a true value when a codebase defines profiles.
	var/default = FALSE

	proc/bake(atom/target)

	proc/prepare(atom/target)

	proc/light(atom/target)

	// Map each connection channel to a DEMIR_CONNECTION_* role mask.
	proc/connections(atom/target)

	// Return the tile regions that the editor must shade for this atom. Each entry is an associative
	// list. The editor ignores unknown keys.
	//
	//     "x", "y", "width", "height"  rectangle with offsets from the atom
	//     "tiles"                      list(list(x, y), ...) offsets for a custom shape
	//     "color"                      hex color, default orange
	//     "fill"                       fill opacity from 0 to 1, default 0.12
	//     "outline"                    border visibility, default 1
	//     "when"                       DEMIR_HIGHLIGHT_* mask, default selected
	//     "label"                      text above the region
	proc/highlights(atom/target)

	// Draw one editor frame for the selected atom. The target is null when the editor has no selection.
	// imgui_* procedures can run only from this hook.
	//
	// The editor retains widget values. A widget first returns the value supplied by the profile.
	// After an edit, it returns the edited value on each frame. A button returns true on the frame
	// after the user presses it.
	//
	// The runtime keeps profile writes only when an interaction changes the heap. Idle frames restore
	// their writes. Store panel state in profile variables:
	//
	//   /datum/demir/example
	//     var/smooth = TRUE
	//
	//     ui(atom/target)
	//       smooth = imgui_checkbox("Smooth walls", smooth)
	//
	// Call demir_rebake() after a state change. The requested hooks then read the new state. New()
	// does not run again.
	proc/ui(atom/target)

// Return the active profile. An ordinary procedure can use it to read profile state. Return null
// outside a bake.
/proc/demir_profile()
	set __demir_intrin = 720

// Enable the Node tool for `subtype` and its descendants. `blocker` accepts one type or a list of
// types. The router cannot cross those types or their descendants. Pass null when no blocker is
// required. `orientable_subtype` names the descendant subtree whose `dir` may change to fit a
// route. Register each participating type's directional openings with demir_node_orientation().
// Call this procedure only from the profile's New(). Repeat a subtype to add blockers.
// A conflicting non-null orientable subtype leaves that registration unchanged.
/proc/demir_node_group(subtype, blocker, orientable_subtype = null)
	set __demir_intrin = 721

// Map one icon `dir` of `subtype` to its cardinal opening mask (NORTH | SOUTH | EAST | WEST).
// Call from the profile's New(), after demir_node_group(). Repeating a type and `dir`
// replaces its previous mask. A mask of zero explicitly leaves that orientation disconnected.
/proc/demir_node_orientation(subtype, direction, openings)
	set __demir_intrin = 726

// Rebaking

// Add `type` and its descendants to the bit group `group`. demir_rebake() can select that group.
// The profile defines the meaning of each bit. The editor only matches the bits. A type can belong
// to multiple groups. Call this procedure only from the profile's New().
/proc/demir_define_group(group, type)
	set __demir_intrin = 719

// Combine these flags to select the derived data for demir_rebake().
#define DEMIR_BAKE_APPEARANCE 1
#define DEMIR_BAKE_LIGHT 2
#define DEMIR_BAKE_HIGHLIGHT 4

// Request new derived data after ui() changes profile state. Call this procedure only from ui().
// The runtime acts on the request only when it keeps the frame's writes.
//
// `groups` selects placements by their demir_define_group() bits. Zero selects every placement.
// Multiple calls in one frame combine their flags and groups.
//
// Profile state does not affect the map until ui() requests the required derived data.
/proc/demir_rebake(kinds, groups = 0)
	set __demir_intrin = 718
