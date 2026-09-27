/obj/parent
/obj/parent/child

/proc/test()
    var/obj/parent/child/path = /obj/parent/child
    var/obj/instance = new /obj/parent/child
    return "[initial(path.parent_type)] [initial(path.type)] [initial(instance.parent_type)]"
