// Next available intrinsic indices: datum 1005, list 1110.
// PLEASE UPDATE THIS WHEN YOU ADD NERW ID!!!

// Root datum

/datum
	var/type
	var/parent_type
	var/list/vars
	var/tag

	proc/New()
		set __demir_intrin = "DatumNew"

	proc/Del()
		set __demir_intrin = "DatumDel"

	proc/Topic(href, href_list)
		set __demir_intrin = "DatumTopic"

	proc/Read(savefile/F)
		set __demir_intrin = "DatumRead"

	proc/Write(savefile/F)
		set __demir_intrin = "DatumWrite"

// Lists

/list
	var/len
	var/const/type = /list

	proc/New(Size)
	proc/Add(Item1)
		set __demir_intrin = "ListAdd"
	proc/Copy(Start = 1, End = 0)
		set __demir_intrin = "ListCopy"
	proc/Cut(Start = 1, End = 0)
		set __demir_intrin = "ListCut"
	proc/Find(Elem, Start = 1, End = 0)
		set __demir_intrin = "ListFind"
	proc/Insert(Index, Item1)
		set __demir_intrin = "ListInsert"
	proc/Join(Glue, Start = 1, End = 0)
		set __demir_intrin = "ListJoin"
	proc/Remove(Item1)
		set __demir_intrin = "ListRemove"
	proc/RemoveAll(Item1)
		set __demir_intrin = "ListRemoveAll"
	proc/Swap(Index1, Index2)
		set __demir_intrin = "ListSwap"
	proc/Splice(Start = 1, End = 0, Item1, ...)
		set __demir_intrin = "ListSplice"

/alist
	parent_type = /list
	var/const/type = /alist
	proc/New(items)
