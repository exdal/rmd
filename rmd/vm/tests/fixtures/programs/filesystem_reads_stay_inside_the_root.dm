/proc/inside()
	return file2text("tips.txt")

/proc/dotted()
	return file2text("sub/../tips.txt")

/proc/backslash()
	return file2text("sub\\inner.txt")

/proc/escaping()
	return file2text("../outside.txt")

/proc/deep_escaping()
	return file2text("sub/../../outside.txt")

/proc/absolute()
	return file2text("/etc/passwd")
