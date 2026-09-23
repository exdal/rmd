
/proc/value(number = 4)
    try
        number += 1
    catch
        number = -100
    return number
/proc/test()
    return value()
