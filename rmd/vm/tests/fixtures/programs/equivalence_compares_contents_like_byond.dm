/datum/probe

/proc/args_probe(a, b)
    return args ~= list(1, 2)

/proc/test()
    var/datum/probe/p = new
    var/datum/probe/q = new
    var/list/inner = list(1)
    var/list/l = list(1, 2)
    var/text = "[("abc" ~= "abc")][("abc" ~= "ABC")][("abc" ~! "ABC")][("1" ~= 1)][("" ~= null)][(null ~= 0)]"
    var/lists = "[(l ~= l)][(list(1, 2) ~= list(1, 2))][(list(1, 2) ~! list(1, 2))][(list(1, 2) ~= list(2, 1))][(list(1, 2) ~= list(1, 2, 3))][(list("a") ~= list("A"))]"
    var/nested = "[(list(inner) ~= list(inner))][(list(list(1)) ~= list(list(1)))]"
    var/assoc = "[(list("a" = 1) ~= list("a" = 1))][(list("a" = 1) ~= list("a" = 2))][(list("a" = 1) ~= list("a"))][(list() ~= list())][(list() ~= null)]"
    var/kinds = "[(alist("a" = 1) ~= list("a" = 1))][args_probe(1, 2)]"
    var/matrices = "[(matrix() ~= matrix())][(matrix() ~= matrix(2, MATRIX_SCALE))][(matrix() ~= list(1, 0, 0, 0, 1, 0))]"
    var/datums = "[(p ~= p)][(p ~= q)]"
    return "[text];[lists];[nested];[assoc];[kinds];[matrices];[datums]"
