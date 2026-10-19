/proc/spawn_delay(list/events, delay)
	events += "delay"
	return delay

/proc/spawn_sequence(delay)
	var/list/events = list("before")
	spawn(spawn_delay(events, delay))
		events += "spawn"
	events += "after"
	return events.Join(",")

/proc/spawn_order()
	return "[spawn_sequence(-2)];[spawn_sequence(-1)];[spawn_sequence(0)];[spawn_sequence(1)]"

/proc/spawn_default_delays()
	var/list/events = list("before")
	spawn
		events += "implicit"
	spawn()
		events += "empty"
	spawn(0)
		events += "zero"
	spawn(1)
		events += "positive"
	events += "after"
	return events.Join(",")

/proc/spawn_return_side_effect(list/shared)
	shared[2] = 1
	return 500

/proc/spawn_frame(value = 7)
	var/local = value
	var/list/shared = list(0, 0)
	. = 42
	spawn(-1)
		local = 99
		value = 100
		. = 88
		shared[1] = local
		return spawn_return_side_effect(shared)
		shared[1] = 0
	return local * 10000 + shared[1] * 100 + shared[2] * 10 + . + value

/proc/spawn_nested()
	var/local = 1
	var/list/shared = list()
	. = 10
	spawn(-1)
		local = 2
		. = 20
		spawn(-1)
			local = 3
			. = 30
			shared += local + .
			return
		shared += local + .
	shared += local + .
	return shared.Join(",")

/proc/spawn_returns_in_loops()
	var/list/shared = list(0, 0)
	for(var/i in 1 to 3)
		spawn(-1)
			if(i == 2)
				return
			for(var/j in 1 to 3)
				if(j == 2)
					continue
				shared[1] += i
				break
		shared[2]++
	return shared[1] * 10 + shared[2]

/proc/spawn_throws()
	var/local = 7
	var/list/shared = list(0)
	. = 42
	try
		spawn(-1)
			local = 8
			. = 99
			shared[1] = 123
			throw 5
			shared[1] = 0
	catch(var/error)
		return -1
	return local * 1000 + . + shared[1]

/proc/spawn_sleeps()
	var/list/events = list()
	spawn(-1)
		events += "a"
		sleep(1)
		events += "b"
	events += "after"
	return events.Join(",")

/proc/spawn_returns_in_try()
	var/list/events = list()
	try
		spawn(-1)
			try
				return
			catch
				events += "spawn caught"
		events += "after"
		throw "x"
	catch
		events += "parent caught"
	return events.Join(",")

/proc/spawn_dot_in_try()
	var/list/events = list()
	. = 1
	spawn(-1)
		try
			. = 5
			throw 1
		catch
			events += "[.]"
		events += "[.]"
	events += "[.]"
	return events.Join(",")

/datum/spawn_parent
	var/stored = 0
	proc/take(value)
		stored = value

/datum/spawn_parent/spawn_child
	take(value)
		spawn(-1)
			value = 5
			..()

/proc/spawn_super_uses_copies()
	var/datum/spawn_parent/spawn_child/child = new
	child.take(1)
	return child.stored

/proc/spawn_shell_fault()
	spawn(-1)
		shell("ls")
	return 1

/proc/spawn_try_inside()
	var/local = 7
	var/list/shared = list(0)
	spawn(-1)
		try
			local = 8
			throw 1
		catch(var/error)
			shared[1] = local
	return shared[1] * 10 + local

/proc/spawn_static()
	var/static/counter = 0
	spawn(-1)
		counter++
	return counter

/datum/spawn_target
	var/value = 0
	proc/change()
		spawn(-1)
			value = 123
		return value

/proc/spawn_object()
	var/datum/spawn_target/target = new
	return target.change()
