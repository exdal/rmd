/proc/t(list/L, a, b)
	var/n = 0
	outer:
		for(var/mob/M in L)
			for(var/i = 1 to 10)
				if(a && b)
					continue outer
				else if(a || i > 3)
					break
				n += i
	switch(n)
		if(1 to 5)
			n = a ? 1 : 2
		else
			n = 0
	return n
