/datum/a
/datum/b
/datum/holder
    var/datum/b/stored

/datum/holder/proc/fill(list/things)
    src.stored = locate() in things

/proc/test()
    var/datum/b/decoy = new
    var/datum/b/wanted = new
    var/list/things = list(new /datum/a, wanted)
    var/datum/b/declared = locate() in things
    var/datum/b/assigned
    assigned = locate() in things
    var/datum/b/missing = locate() in list(new /datum/a)
    var/datum/holder/holder = new
    holder.fill(things)
    return "[decoy != wanted] [declared == wanted] [assigned == wanted] [isnull(missing)] [holder.stored == wanted]"
