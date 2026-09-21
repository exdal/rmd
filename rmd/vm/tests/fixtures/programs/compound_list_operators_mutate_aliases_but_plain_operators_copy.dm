
/proc/test()
    var/list/original = list(1, 2)
    var/list/alias = original
    alias += 3
    var/list/copy = original + 4
    return length(original) * 100 + length(alias) * 10 + length(copy)
