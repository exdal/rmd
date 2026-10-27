/proc/box(x1, y1, x2 = x1, y2 = y1)
	return "[x1],[y1],[x2],[y2]"
/proc/test()
	return "[box(1, 2)];[box(1, 2, 3)];[box(1, 2, 3, 4)]"
