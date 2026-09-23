
/proc/test()
    var/device_type = 3
    var/list/node_connects[device_type]
    node_connects[1] = 1
    node_connects[2] = 2
    node_connects[3] = 4
    return node_connects.len * 10 + node_connects[3]
