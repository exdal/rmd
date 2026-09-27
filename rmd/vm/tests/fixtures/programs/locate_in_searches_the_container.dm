
/datum/a
/datum/b

/proc/test()
    var/datum/b/wanted = new
    var/list/things = list(new /datum/a, wanted)
    var/datum/b/found = locate(/datum/b) in things
    var/missing = locate(/datum/b) in list(new /datum/a)
    var/datum/b/assigned
    assigned = locate(/datum/b) in things
    return "[found == wanted] [isnull(missing)] [istype(things ? locate(/datum/a) in things : null, /datum/a)] [assigned == wanted]"
