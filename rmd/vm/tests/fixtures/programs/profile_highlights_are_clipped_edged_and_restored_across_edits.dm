
/obj/port
    var/span = 0
/datum/demir/test/highlights(atom/target)
    if(!istype(target, /obj/port))
        return
    var/obj/port/port = target
    target.name = "highlight hook writes are rolled back"
    return list(
        list("x" = -1, "y" = -1, "width" = port.span, "height" = port.span,
             "color" = "#00ff00", "fill" = 0.5, "when" = 4, "label" = "span"),
        list("tiles" = list(list(0, 2), list(1, 2))),
    )
