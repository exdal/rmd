
/proc/test()
    var/value = 1
    try
        value = 2
        throw 9
    catch(var/error)
        value += error
    return value
