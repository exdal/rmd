
/proc/select_and_increment(condition, a, b)
    var/value
    if(condition)
        value = a + 1
    else
        value = b + 1
    return value
/proc/test()
    return select_and_increment(1, 10, 20) * 100 + select_and_increment(0, 10, 20)
