/proc/greet(a = 5, b = "x")
	return "[a][b]"
/proc/test_defaults()
	return "[greet()],[greet(1)],[greet(1, "y")]"
/proc/repeat(n)
	var/text = ""
	for(var/i = 0, i < n, i++)
		text += "ab"
	return text
/proc/test_repeat()
	return "[repeat(0)],[repeat(3)]"
/proc/test_two_constants()
	var/list/pair = list("a", "b")
	return "[pair[1]][pair[2]]"
/proc/choose(x)
	var/answer
	if(x)
		answer = "yes"
	else
		answer = "no"
	return answer
/proc/test_choose()
	return "[choose(1)],[choose(0)]"
/proc/guarded(x)
	var/list/found = list("key" = x)
	try
		throw "boom"
	catch(var/error)
		return "[error]-[x + 1]-[found["key"]]"
/proc/test_guarded()
	return guarded(2)
/proc/minus(x)
	var/doubled = x * 2
	return 10 - doubled
/proc/test_minus()
	return minus(3)
/proc/shift(n)
	var/previous = 0
	var/current = 5
	while(n > 0)
		previous = current
		current = 7
		n--
	return "[previous],[current]"
/proc/test_shift()
	return "[shift(0)],[shift(1)],[shift(2)]"
