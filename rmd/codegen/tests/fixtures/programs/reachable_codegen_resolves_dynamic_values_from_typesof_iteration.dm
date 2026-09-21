
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/proc/typesof(Type1, Type2)
    set __demir_intrin = 377
/datum/controller/global_vars/proc/Initialize()
    var/list/global_procs = typesof(/datum/controller/global_vars/proc)
    for(var/proc_path in global_procs)
        call(src, proc_path)()
/proc/unrelated()
    return 1
