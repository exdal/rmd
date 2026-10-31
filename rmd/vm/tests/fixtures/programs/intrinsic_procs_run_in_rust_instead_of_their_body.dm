
/world
    proc/file2list(File, Separator)
        set __demir_intrin = "WorldFile2List"
    proc/IsBanned(key, address, computer_id, type)
        set __demir_intrin = "WorldIsBanned"
    proc/Reboot(reason)
        set __demir_intrin = "WorldReboot"
/proc/lines()
    return world.file2list("tips.txt")
/proc/banned()
    return world.IsBanned("key")
/proc/reboot()
    return world.Reboot()
