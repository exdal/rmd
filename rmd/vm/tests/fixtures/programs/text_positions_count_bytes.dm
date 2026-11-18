/proc/indexing()
	var/w = "çay1"
	return json_encode(list(w[1], w[2], w[3], w[4], w[5]))

/proc/lengths()
	var/w = "çay1"
	return "[length(w)]|[length_char(w)]"

/proc/ascii_codes()
	var/w = "çay1"
	return "[text2ascii(w, 1)]|[text2ascii(w, 2)]|[text2ascii(w, 3)]|[text2ascii_char(w, 2)]"

/proc/copies()
	var/w = "çay1"
	return json_encode(list(copytext(w, 1, 2), copytext(w, 2), copytext(w, 1, 3), copytext_char(w, 1, 2)))

/proc/searches()
	var/w = "çay1"
	return "[findtext(w, "a")]|[findtext_char(w, "a")]|[findtext(w, "y", 4)]"

/proc/searches_from_the_end()
	var/w = "çaça"
	return "[findlasttext(w, "a")]|[findlasttext_char(w, "a")]|[findtextEx_char(w, "A")]|[findlasttextEx_char(w, "a")]"

/proc/replaced()
	var/w = "çay1"
	return json_encode(replacetext(w, "a", "b"))

/proc/split()
	var/w = "çay1"
	return json_encode(splittext(w, "a"))

/proc/uppercased()
	var/w = "çay1"
	return "[json_encode(uppertext(w))]|[length(uppertext(w))]"

/proc/out_of_range()
	var/w = "çay1"
	return "[w[6]]|[text2ascii(w, 9)]|[json_encode(copytext(w, 3, 99))]"
