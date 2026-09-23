
/proc/test()
    var/image/source = new
    source.icon_state = "src"
    var/mutable_appearance/MA = mutable_appearance(source)
    return MA.icon_state
