
/proc/test()
    var/value = 0
    do {
        done: {
            value = 1
            break done
            value = 2
        }
    } while(FALSE)
    return value
