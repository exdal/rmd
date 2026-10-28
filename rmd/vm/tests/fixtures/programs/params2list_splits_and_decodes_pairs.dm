/proc/test()
    var/list/params = params2list("left=-14,0;right=13,0&flag&name=a+b%21&left=1,2")
    return "[length(params)]:[params["left"]]:[params["right"]]:[isnull(params["flag"])]:[params["name"]]"
