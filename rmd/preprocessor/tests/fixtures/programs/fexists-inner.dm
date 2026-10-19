#if fexists(CHILD_LOCAL_FILE) && !fexists("root-only.dm")
#include "local.dm"
#else
wrong_directory
#endif
