/datum/thing

/proc/datum_text()
	var/datum/thing/D = new
	return "[D]|[addtext(D)]|[jointext(list(D, 1e8), ",")]|[text("[D]")]"
