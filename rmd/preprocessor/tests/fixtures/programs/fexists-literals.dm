#if fexists("+secret/foo.dm")
#include "+secret/foo.dm"
#else
wrong_branch
#endif
#if fexists("+secret/missing.dm")
wrong_branch
#else
missing_file
#endif
#if fexists("+secret\foo.dm")
backslash_path
#endif
#if !fexists("+secret")
not_a_file
#endif
