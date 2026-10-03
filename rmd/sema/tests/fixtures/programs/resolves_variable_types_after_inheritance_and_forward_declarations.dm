/datum/base
	var/mob/target
	var/list/items[2]
	var/list/datum/material/materials
	var/title = null as text|null
	var/choice = null as text|num|null
	var/mob/nullable_target as null
	var/health = 1
	var/mixed = 1
	var/path_value = /mob
	var/flag = FALSE
	var/loose_flag = 0
	var/mixed_flag = TRUE
	var/counted = FALSE
/datum/child
	parent_type = /datum/base
	health = 2
	mixed = "other"
	flag = TRUE
	loose_flag = 1
	mixed_flag = 1
	counted = 2
/mob
