#ifdef __DEMIR_BAKE__

#define EXAMPLE_EXT "example_ext"

/datum/demir/example_ext
	default = TRUE
	var/tint

	New()
		tint = load_ext(EXAMPLE_EXT, "tint")

	bake(atom/target)
		if(!istype(target, /turf/open/floor))
			return

		target.color = call_ext(tint)("[target.x]", "[target.y]")
		target.desc = "[call_ext(EXAMPLE_EXT, "greet")(target.name)] (call [call_ext(EXAMPLE_EXT, "calls")()])"

#endif
