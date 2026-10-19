/mob/proc/plain()
    return __PROC__
/mob/verb/spoken()
    return __PROC__
/mob/child/plain()
    return __PROC__
/mob/child/spoken()
    return __PROC__
/proc/test()
    var/mob/base = new
    var/mob/child/child = new
    return "[base.plain()],[base.spoken()],[child.plain()],[child.spoken()],[/mob/proc/plain],[/mob/verb/spoken],[base.spoken() == /mob/verb/spoken]"
