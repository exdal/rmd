
/proc/select_and_increment(condition, a, b)
    var/value
    if(condition)
        value = a + 1
    else
        value = b + 1
    return value
