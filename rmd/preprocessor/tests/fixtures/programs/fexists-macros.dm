#define OPTIONAL_FILE "+secret/foo.dm"
#define OPTIONAL_ALIAS OPTIONAL_FILE
#define IDENTITY(x) x
#define OPTIONAL_CALL(x) IDENTITY(x)
#define HAS_OPTIONAL(x) fexists(x)
#if fexists(OPTIONAL_FILE) && defined(OPTIONAL_FILE)
object_macro
#endif
#if fexists(OPTIONAL_ALIAS)
chained_macro
#endif
#if fexists(OPTIONAL_CALL(OPTIONAL_ALIAS))
function_macro
#endif
#if HAS_OPTIONAL(OPTIONAL_FILE)
wrapped_fexists
#endif
#if !fexists("+secret/missing.dm") && (defined(OPTIONAL_FILE) || 0)
negated_combination
#endif
#if 0
wrong_branch
#elif fexists(OPTIONAL_CALL(OPTIONAL_ALIAS)) && !defined(DISABLE_OPTIONAL)
elif_macro
#else
wrong_branch
#endif
