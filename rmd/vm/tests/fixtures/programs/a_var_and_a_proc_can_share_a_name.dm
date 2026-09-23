
/datum/rock
    var/list/edges = null

/datum/rock/proc/edges()
    src.edges = list()
    src.edges += "north"
    edges += "south"
    return length(src.edges)

/proc/test()
    var/datum/rock/rock = new
    return "[rock.edges()] [length(rock.edges)] [rock.edges[2]]"
