#define CHILD_LOCAL_FILE "local.dm"
#include "nested/inner.dm"
#if fexists("root-only.dm") && !fexists("local.dm")
restored_directory
#else
wrong_directory
#endif
