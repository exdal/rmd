/proc/immediates(x, list/L)
	x += 2
	if(x < 3)
		return 5
	if(L[2])
		return null
	return 1 - x
