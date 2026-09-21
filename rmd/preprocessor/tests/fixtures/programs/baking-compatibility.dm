#if (DM_VERSION < 516 || DM_BUILD < 1659) && !defined(SPACEMAN_DMM)
#error too old
#endif
/proc/test()
	return 1
