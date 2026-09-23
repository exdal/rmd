
/proc/axis(d)
    switch(d)
        if(1, 2)
            return "ns"
        if(4, 8)
            return "ew"
    return "none"
/proc/test()
    return "[axis(1)] [axis(2)] [axis(4)] [axis(8)] [axis(3)]"
