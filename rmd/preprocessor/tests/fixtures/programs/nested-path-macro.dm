#define DEFINE #define
#define MAKE(_NAME) /datum/base/##_NAME {} DEFINE _NAME(x) x
#define OTHER(_NAME) /datum/other/##_NAME
#define WRAP(_PATH, _NAME) _PATH(_NAME) {}
MAKE(OFFSETS)
WRAP(OTHER, OFFSETS)
