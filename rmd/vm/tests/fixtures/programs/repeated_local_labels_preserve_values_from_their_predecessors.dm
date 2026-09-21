
/proc/test()
    var/value = 7
    do {
        done: {
            break done
        }
    } while(FALSE)
    var/after_first = value
    do {
        done: {
            break done
        }
    } while(FALSE)
    return after_first
