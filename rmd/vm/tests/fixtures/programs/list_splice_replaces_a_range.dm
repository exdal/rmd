
/proc/test()
    var/list/L = list("a", "b", "c")
    L.Splice(2, 3, "x", "y")
    return L.Join("")
