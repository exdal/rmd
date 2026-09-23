
/proc/raiser()
    throw 42
/proc/test()
    try
        raiser()
    catch(var/value)
        return value + pick(list(7))
