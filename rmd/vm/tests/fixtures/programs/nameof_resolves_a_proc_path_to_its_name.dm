
/proc/nameof(X)
    set __demir_intrin = "Nameof"
/proc/work()
    return 1
/proc/test()
    return nameof(/proc/work)
