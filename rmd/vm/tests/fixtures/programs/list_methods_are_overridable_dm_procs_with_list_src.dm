
/list/Add(Item1)
    return src.len * 10 + Item1
/proc/test()
    var/list/L = list(1, 2)
    return L.Add(7) * 10 + L.len
