/proc/t(mean, stddev)
	var/cached
	var/r1
	var/r2
	var/working
	if(cached != null)
		r1 = cached
		cached = null
	else
		do
			r1 = rand(-10000, 10000) / 10000
			r2 = rand(-10000, 10000) / 10000
			working = r1 * r1 + r2 * r2
		while(working >= 1 || working == 0)
		working = sqrt(-2 * log(working) / working)
		r1 *= working
		cached = r2 * working
	return mean + stddev * r1
