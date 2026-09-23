var/constructed = 0

/datum/item/New()
    constructed += 1

/proc/generate_items(list/items)
    for(var/item_type in items)
        for(var/i in 1 to items[item_type])
            new item_type

/proc/test()
    generate_items(list(
        /datum/item = 3,
    ))
    return constructed
