/obj/lower
	name = "widget"

/obj/upper
	name = "Widget"

/obj/improper
	name = "\improper Widget"

/obj/proper
	name = "\proper widget"

/obj/empty
	name = ""

/mob/person
	name = "Bob"

/proc/atoms()
	var/obj/lower/L = new
	var/obj/upper/U = new
	var/obj/improper/I = new
	var/obj/proper/P = new
	var/obj/empty/E = new
	var/mob/person/B = new
	return list(L, U, I, P, E, B)

/proc/interpolated()
	var/list/a = atoms()
	return "[a[1]] | [a[2]] | [a[3]] | [a[4]] | [a[5]] | [a[6]]"

/proc/prefixed()
	var/list/a = atoms()
	return "x [a[1]] | x [a[2]] | x [a[3]] | x [a[4]] | x [a[5]] | x [a[6]]"

/proc/encoded()
	return json_encode(atoms())

/proc/names()
	var/list/a = atoms()
	var/obj/lower/L = a[1]
	var/obj/improper/I = a[3]
	var/obj/proper/P = a[4]
	return json_encode(list(L.name, I.name, P.name))

/proc/joined()
	var/list/a = atoms()
	return jointext(list(a[1], a[2]), ",")
