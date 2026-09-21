
/obj/source
    var/channel
/obj/target
    var/channel
/obj/both
    var/channel
/datum/demir/test/connections(atom/target)
    var/list/connections = list()
    if(istype(target, /obj/source))
        var/obj/source/source = target
        connections[source.channel] = 1
    else if(istype(target, /obj/target))
        var/obj/target/destination = target
        connections[destination.channel] = 2
    else if(istype(target, /obj/both))
        var/obj/both/both = target
        connections[both.channel] = 3
    target.name = "connection hook writes are rolled back"
    return connections
