/proc/integers()
	return "[1e7]|[1234567]|[16777216]|[16777217]|[999999]|[1000000]|[-0]|[255]|[-3]"

/proc/fractions()
	return "[0.1]|[1/3]|[123456.7]|[-2.25]|[0.0001]"

/proc/exponents()
	return "[1e-5]|[1e-7]|[1e8]|[2147483647]|[2147483648]|[1e10]|[-1e8]|[1e30]|[3.4e38]"

/proc/num2text_matches()
	return "[num2text(1e8)]|[num2text(1/3)]|[num2text(7)]"

/proc/text2num_round_trip()
	return "[text2num("1e+08")]|[text2num("1e-05")]|[text2num("[1/3]")]"

/proc/concatenated()
	return addtext("a", 1e8, 0.5)
