
/proc/value(a = 2, b = a + 3)
    return b
/proc/test()
    return value(4) * 10 + value()
