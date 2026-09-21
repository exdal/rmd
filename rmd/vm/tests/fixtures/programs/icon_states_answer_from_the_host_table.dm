
/proc/test()
    var/icon/wrapped = icon('Icons\Walls.dmi')
    var/list/found = icon_states('icons/walls.dmi')
    return "[length(found)] [found[1]] [found[2]] [length(wrapped.IconStates())] [length(icon_states('missing.dmi'))]"
