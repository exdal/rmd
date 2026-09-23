
/proc/test()
    var/image/I = image('a.dmi', null, "s", 3, 1, 4, 5, 6, 7)
    return I.pixel_w * 10 + I.pixel_z
