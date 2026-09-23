
/alist
    var/len
    proc/New(items)
/proc/islist(L)
    set __demir_intrin = 279
/proc/test()
    var/alist/A = new(list("a", "b"))
    return islist(A) + A.len
