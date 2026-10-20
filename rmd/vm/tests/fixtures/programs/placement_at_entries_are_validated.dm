/datum/demir/neither

/datum/demir/neither/New()
	..()
	demir_place_modular(list(list("maps" = list())))

/datum/demir/both

/datum/demir/both/New()
	..()
	demir_place_modular(list(list("root" = world, "at" = list(1, 1, 1), "maps" = list())))

/datum/demir/two_numbers

/datum/demir/two_numbers/New()
	..()
	demir_place_modular(list(list("at" = list(1, 1), "maps" = list())))

/datum/demir/not_a_list

/datum/demir/not_a_list/New()
	..()
	demir_place_modular(list(list("at" = "1,1,1", "maps" = list())))

/datum/demir/not_a_number

/datum/demir/not_a_number/New()
	..()
	demir_place_modular(list(list("at" = list(1, "y", 1), "maps" = list())))

/datum/demir/zero

/datum/demir/zero/New()
	..()
	demir_place_modular(list(list("at" = list(1, 0, 1), "maps" = list())))

/datum/demir/fractional

/datum/demir/fractional/New()
	..()
	demir_place_modular(list(list("at" = list(1, 1.5, 1), "maps" = list())))

/datum/demir/duplicate_in_call

/datum/demir/duplicate_in_call/New()
	..()
	demir_place_modular(list(list("at" = list(2, 2, 2), "maps" = list()), list("at" = list(2, 2, 2), "error" = "again")))

/datum/demir/duplicate_across_calls

/datum/demir/duplicate_across_calls/New()
	..()
	demir_place_modular(list(list("at" = list(2, 2, 2), "maps" = list())))
	demir_place_modular(list(list("at" = list(2, 2, 2), "maps" = list())))
