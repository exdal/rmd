
/proc/test(n)
    var/i = 0
    while(i < n)
        var/next = i + 1
        world.log << i
        i = next
    return i
