
/proc/test()
    var/value = 0
    do {
        done: {
            value += 1
            break done
        }
    } while(FALSE)
    do {
        done: {
            value += 1
            break done
        }
    } while(FALSE)
    return value
