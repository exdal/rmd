/datum/thing
/obj/widget

/proc/empty_list()
	return json_encode(list())

/proc/integers()
	return json_encode(list(5, -3, 0, 255, 1234567, 16777217))

/proc/fractions()
	return json_encode(list(0.5, -2.25, 0.1, 1/3, 0.0001, 1e-7))

/proc/large()
	return json_encode(list(1e6, 1e7, 1e30, 3.4e38))

/proc/exponents()
	return json_encode(list(1e8, 2147483647, 2147483648, 1e10, -1e8))

/proc/non_finite()
	var/inf = 1e38 * 10
	return json_encode(list(inf, -inf, inf - inf))

/proc/null_value()
	return json_encode(null)

/proc/null_entry()
	return json_encode(list(null, 1))

/proc/type_path()
	return json_encode(/obj/widget)

/proc/datum_value()
	var/datum/thing/D = new
	return json_encode(D)

/proc/datum_in_list()
	var/datum/thing/D = new
	return json_encode(list(D))

/proc/associations()
	return json_encode(list("a" = 1, "b" = list(2, 3)))

/proc/key_without_a_value()
	return json_encode(list("a", "b" = 2))

/proc/typed_keys()
	var/datum/thing/D = new
	var/list/keyed = list()
	keyed[D] = 1
	keyed[/obj/widget] = 2
	return json_encode(keyed)

/proc/nested()
	return json_encode(list(list(), list(list(1))))

/proc/escapes()
	return json_encode("quote \" backslash \\ slash / tab \t newline \n unicode ç")

/proc/control_byte()
	return json_encode(ascii2text(1))

/proc/control_characters()
	return json_encode(ascii2text(8) + ascii2text(12) + ascii2text(13))

/proc/delete_character()
	return json_encode(ascii2text(127))

/proc/surrogate_pair()
	return json_encode("😀")

/proc/pretty_object()
	return json_encode(list("a" = 1, "b" = list(2)), JSON_PRETTY_PRINT)

/proc/pretty_empty()
	return json_encode(list(), JSON_PRETTY_PRINT)

/proc/pretty_empty_member()
	return json_encode(list("a" = list()), JSON_PRETTY_PRINT)

/proc/pretty_array()
	return json_encode(list(list("a" = 1, "b" = 2), 3), JSON_PRETTY_PRINT)

/proc/strict_flag_is_ignored()
	return json_encode(list(1), JSON_ENCODE_STRICT)

/proc/round_trip()
	var/list/original = list("name" = "q\"uote", "nested" = list(1, 2.5, "three"), "empty" = list())
	var/list/decoded = json_decode(json_encode(original))
	return "[decoded["name"]]|[decoded["nested"][2]]|[decoded["nested"][3]]|[length(decoded["empty"])]|[json_encode(decoded) == json_encode(original)]"

/proc/self_containing()
	var/list/loop = list()
	loop += list(loop)
	return json_encode(loop)
