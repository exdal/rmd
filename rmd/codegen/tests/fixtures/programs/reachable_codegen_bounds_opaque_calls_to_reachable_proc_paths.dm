
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/proc/live()
    return 1
/proc/dead()
    return 2
/proc/invoke(callback)
    return call(callback)()
/proc/entry(callback = /proc/live)
    return invoke(callback)
