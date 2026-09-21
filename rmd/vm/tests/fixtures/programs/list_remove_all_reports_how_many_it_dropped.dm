
/proc/test()
    var/list/L = list(1, null, 2, null, null, 3)
    var/dropped = L.RemoveAll(null)
    return dropped * 100 + L.len * 10 + L[1]
