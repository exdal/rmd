
/proc/swap(a, b, n)
    while(n > 0)
        var/t = a
        a = b
        b = t
        n--
    return "[a][b]"
/proc/rotate(a, b, c, n)
    while(n > 0)
        var/t = a
        a = b
        b = c
        c = t
        n--
    return "[a][b][c]"
/proc/test()
    return "[swap(1, 2, 1)],[swap(1, 2, 2)],[rotate(1, 2, 3, 1)],[rotate(1, 2, 3, 2)]"
