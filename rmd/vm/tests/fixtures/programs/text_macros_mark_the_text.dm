/proc/lengths()
	var/a = "\improper Widget"
	var/b = "\proper widget"
	return "[length(a)]|[length(b)]|[length("Widget")]"

/proc/comparisons()
	var/a = "\improper Widget"
	var/b = "\proper widget"
	return "[a == "Widget"]|[a == "\improper Widget"]|[b == "widget"]"

/proc/copies()
	var/a = "\improper Widget"
	return "[json_encode(copytext(a, 1, 3))]|[json_encode(copytext(a, 3))]"

/proc/searches()
	var/a = "\improper Widget"
	var/b = "\proper widget"
	return "[findtext(a, "W")]|[findtext(b, "w")]"

/proc/concatenation()
	var/a = "\improper Widget"
	return "[json_encode(a + "!")]|[json_encode("x [a]")]"

/proc/ascii_codes()
	var/a = "\improper Widget"
	return "[text2ascii(a, 1)]|[text2ascii(a, 2)]|[text2ascii(a, 3)]|[text2ascii("\proper widget", 2)]"

/proc/uppercased()
	var/a = "\improper Widget"
	return json_encode(uppertext(a))

/proc/whole_words_only()
	return "[length("\properly")]|[length("\improperly")]"

/proc/logged()
	world.log << "\improper Widget"
	return "done"
