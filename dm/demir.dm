#define __DEMIR__

// the BYOND version we support (hopefully)
#define DM_VERSION 516
#define DM_BUILD 1665

#define __DEMIR_COMPAT__
#ifdef __DEMIR_COMPAT__
// mapping tools set this, and codebases gate map editor icon states on it
#define FASTDMM  // we do a little bit of lying
#define SPACEMAN_DMM
#define SpacemanDMM_unlint(X) X
#define SpacemanDMM_debug(X...) X
#endif

// tgstation
// #define CBT  // dont

/atom
	var
		name = null
		icon = null
		icon_state = null
		dir = SOUTH
		layer = 2
		plane = 0
		pixel_x = 0
		pixel_y = 0
		pixel_w = 0
		pixel_z = 0
		color = null
		alpha = 255
		invisibility = 0

/atom/movable
	var
		step_x = 0
		step_y = 0

/area
	layer = AREA_LAYER
	parent_type = /atom
/turf
	layer = TURF_LAYER
	parent_type = /atom
/obj
	layer = OBJ_LAYER
	parent_type = /atom/movable
/mob
	layer = MOB_LAYER
	parent_type = /atom/movable
