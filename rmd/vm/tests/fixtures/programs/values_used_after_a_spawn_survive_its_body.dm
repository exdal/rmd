/proc/test()
    var/list/values = list(1, 2, 3)
    var/kept = length(values) * 10
    spawn(-1)
        var/a = length(values) + 1
        var/b = a * 2
        var/c = b + a
        values += c
        sleep(1)
        values += 99
    return kept + length(values) * 100 + values[4]
