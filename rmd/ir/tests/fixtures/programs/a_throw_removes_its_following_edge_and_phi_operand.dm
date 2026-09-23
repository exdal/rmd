/proc/t(a)
	var/x = 0
	if(a)
		throw 1
		x = 2
	else
		x = 3
	return x
