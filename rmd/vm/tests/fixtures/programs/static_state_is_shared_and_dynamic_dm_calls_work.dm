
/datum/test
    var/static/list/items = list()
    proc/value(a = 9, b = 2)
        var/static/count = 0
        count++
        items += count
        return a + b + length(items)
/proc/test()
    var/datum/test/a = new
    var/datum/test/b = new
    a.value()
    return call(b, "value")(, 3)
