/obj/moved
	transform = matrix(1, 0, -4, 0, 1, -4)

/proc/test()
	var/matrix/M = matrix()
	M.Translate(3, 5)
	M.Scale(2)
	var/matrix/copy = matrix(M)
	copy.Turn(90)
	var/matrix/product = M * 2
	var/matrix/back = M * ~M
	var/matrix/turned = turn(matrix(), 90)
	var/obj/plain = new
	var/obj/moved/moved = new
	return "[M.a],[M.c],[M.f];[copy.a != M.a];[product.c];[back.c],[back.e];[turned.b];[plain.transform.a];[moved.transform.c]"
