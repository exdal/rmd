
/proc/choose(condition)
    var/value = 2
    if(condition)
        value = 1
    return value
/proc/test()
    return choose(1) * 10 + choose(0)
