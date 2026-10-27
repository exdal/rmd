/proc/test(value)
    try
        value += 1
        throw value
    catch
        value += 10
    return value
