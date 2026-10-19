/proc/test(value)
	var/local = value
	spawn(1)
		try
			local = 1
		catch
			local = 2
	return local
