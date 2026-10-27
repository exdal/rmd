/proc/t(a)
    if(a)
        goto inside
    try
        inside:
            return 1
    catch
        return 2
    return 3
