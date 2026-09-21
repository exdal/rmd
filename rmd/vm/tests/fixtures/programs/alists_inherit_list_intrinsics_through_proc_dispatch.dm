
/proc/test()
    var/alist/A = alist("first")
    A.Add("value")
    return istype(A, /alist) * 10 + A.len
