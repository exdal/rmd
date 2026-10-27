
/proc/count(n)
    var/i = 0
    var/seen = ""
    while(i < n)
        var/next = i + 1
        seen += "[i]"
        i = next
    return "[seen]:[i]"
/proc/test()
    return count(3)
