/proc/select(condition, a, b)
	var/value
	if(condition)
		value = a + 1
	else
		value = b + 2
	return value
