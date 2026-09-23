
/world
    proc/file2list(File, Separator)
        set __demir_intrin = 122
    proc/IsBanned(key, address, computer_id, type)
        set __demir_intrin = 112
    proc/Reboot(reason)
        set __demir_intrin = 104
/proc/lines()
    return world.file2list("tips.txt")
/proc/banned()
    return world.IsBanned("key")
/proc/reboot()
    return world.Reboot()
