/obj/prepared
    icon = 'mapping.dmi'
    icon_state = "map"
    color = "#00ff00"
    var/change_sheet = FALSE
    var/bake_state = FALSE
    var/bookkeeping = 0

/obj/prepared/changed_state
    bake_state = TRUE

/obj/prepared/changed_sheet
    change_sheet = TRUE

/datum/demir/test/prepare(atom/target)
    var/obj/prepared/item = target
    item.dir = item.dir == WEST ? EAST : NORTH
    item.alpha = 128
    item.pixel_x = 4
    item.pixel_y = -3
    item.color = null
    if(item.change_sheet)
        item.icon = 'prepared.dmi'
    else
        item.icon_state = "prepared"

/datum/demir/test/bake(atom/target)
    var/obj/prepared/item = target
    // Mutating bookkeeping makes the export compare against the prepared snapshot,
    // even when this hook leaves every appearance field alone.
    item.bookkeeping++
    if(item.bake_state)
        item.icon_state = "baked"
