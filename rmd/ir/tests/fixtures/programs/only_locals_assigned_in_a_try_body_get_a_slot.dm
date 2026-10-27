/proc/test(value)
    var/kept = value
    var/assigned = value
    try
        assigned = kept + 1
    catch
        assigned += kept
    return assigned
