#define BAD_PATH UNDEFINED_PATH
#if fexists(BAD_PATH)
wrong_branch
#else
invalid_path_rejected
#endif
