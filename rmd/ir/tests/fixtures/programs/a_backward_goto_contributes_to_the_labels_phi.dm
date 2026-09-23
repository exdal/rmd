/proc/t(a)
	var/x = 0
	again:
		x += 1
		if(a)
			a = 0
			goto again
		return x
