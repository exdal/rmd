
/proc/test()
    var/value = 0
    done: {
        value = 1
        break done
        value = 2
    }
    return value
