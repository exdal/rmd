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

/area
	layer = AREA_LAYER
/turf
	layer = TURF_LAYER
/obj
	layer = OBJ_LAYER
/mob
	layer = MOB_LAYER
