/proc/fib_iter(n)
    var/a = 0
    var/b = 1
    for(var/i = 0, i < n, i++)
        var/tmp = b
        b = a + b
        a = tmp
    return a

/proc/main()
    world.log << fib_iter(10)
