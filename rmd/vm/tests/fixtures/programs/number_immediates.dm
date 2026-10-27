/proc/arithmetic(x)
	return "[x + 1],[x - 1],[x < 3],[1 - x]"
/proc/test_arithmetic()
	return arithmetic(5)
/proc/add_in_place(x)
	x += 2
	return x
/proc/test_add_in_place()
	return add_in_place(5)
/proc/test_append_in_place()
	var/list/L = list(1, 2)
	var/list/alias = L
	L += 3
	return "[alias.len],[alias[3]]"
/proc/second(list/L, list/missing)
	return "[L[2]],[isnull(missing?[1])]"
/proc/test_second()
	return second(list("a", "b", "c"), null)
/proc/test_return_number()
	return 5
/proc/test_return_null()
	. = 3
	return null
/proc/test_return_dot()
	. = 3
	return
