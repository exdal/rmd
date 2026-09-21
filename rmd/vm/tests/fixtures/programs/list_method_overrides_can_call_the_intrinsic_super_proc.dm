
/list/Add(Item1)
    ..()
    return src.len
/proc/test()
    var/list/L = list(1, 2)
    return L.Add(7) * 10 + L.len
