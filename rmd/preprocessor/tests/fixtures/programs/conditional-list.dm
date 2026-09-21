#define CURRENT 1
#define EXPECTED 1
var/list/items = list(
first,
#if CURRENT == EXPECTED
conditional,
#endif
last,
)
var/after = 1
