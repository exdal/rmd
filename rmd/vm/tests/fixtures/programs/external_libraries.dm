/proc/call_merge()
	return call_ext("merge", "merge")("fee", 1, "fo")

/proc/call_stdcall_merge()
	return call_ext("merge", "@merge")("fi")

/proc/load_merge()
	return load_ext("merge", "merge")

/proc/count_calls()
	var/counter = load_ext("merge", "count")
	var/list/counts = list()
	counts += call_ext(counter)()
	counts += call_ext(counter)()
	counts += call_ext("merge", "count")()
	return jointext(counts, ",")

/proc/call_nothing()
	return isnull(call_ext("merge", "nothing")())

/proc/call_byondapi()
	return call_ext("merge", "byond:merge")()

/proc/call_missing()
	return call_ext("missing", "merge")()

/proc/call_missing_function()
	return call_ext("merge", "absent")()

/proc/call_cwd()
	return call_ext("merge", "cwd")()

/proc/call_touch()
	return call_ext("merge", "touch")()

/proc/trusted()
	return demir_trusted()
