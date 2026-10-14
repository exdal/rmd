/proc/scalars()
	var/list/data = demir_toml_decode("name = \"x\"\nflag = true\nno = false\ncount = 3\nratio = 0.5\n")
	return "[data["name"]]|[data["flag"]]|[data["no"]]|[data["count"]]|[data["ratio"]]"

/proc/nested()
	var/list/data = demir_toml_decode("items = \[1, 2, 3]\n\[rooms.alpha]\nmodules = \[\"a.dmm\", \"b.dmm\"]\n")
	var/list/modules = data["rooms"]["alpha"]["modules"]
	return "[data["items"].len]|[modules.len]|[modules[2]]"

/proc/invalid()
	return demir_toml_decode("name = ")

/proc/caught()
	try
		demir_toml_decode("name = ")
	catch(var/exception/error)
		return "caught: [istype(error, /exception)]|[length(error.name) > 0]"
