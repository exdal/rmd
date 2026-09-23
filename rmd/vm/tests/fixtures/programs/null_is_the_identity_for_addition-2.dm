
/datum/thing
    var/tag_name = "x"
/proc/test()
    var/list/entries = list()
    var/datum/thing/thing = new
    entries["4"] += thing
    var/datum/thing/stored = entries["4"]
    return stored.tag_name
