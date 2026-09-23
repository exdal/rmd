
/obj/thing
    var/global/shared[8]
    var/sized[3]
    var/list/empty[]

/proc/test()
    var/obj/thing/first = new
    var/obj/thing/second = new
    first.sized[1] = 1
    first.shared[1] = 2
    return "[length(first.sized)] [length(first.shared)] [length(first.empty)] [second.sized[1]] [second.shared[1]]"
