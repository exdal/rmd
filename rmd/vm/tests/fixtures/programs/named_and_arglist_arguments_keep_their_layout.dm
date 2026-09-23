
/proc/value(a, b, c)
    return a * 100 + b * 10 + c
/proc/forward(list/arguments)
    return value(arglist(arguments))
/proc/test()
    return forward(list("c" = 3, "a" = 1, "b" = 2))
