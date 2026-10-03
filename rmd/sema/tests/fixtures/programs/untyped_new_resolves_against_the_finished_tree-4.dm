/datum/tracy
/datum/base
	var/datum/tracy/inherited
/datum/holder
	parent_type = /datum/base
	proc/go()
		inherited = new
