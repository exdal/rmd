
/datum
    var/value = 1
    var/list/items = list("old")
/proc/test(datum/target)
    target.value = 2
    target.items.Add("new")
    new /datum
    while(1)
        target.value++
