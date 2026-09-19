///
/// DATUM
///

/datum
	var/type
	var/parent_type
	var/list/vars
	var/tag

	proc/New()
		set __demir_intrin = 1000

	proc/Del()
		set __demir_intrin = 1001

	proc/Topic(href, href_list)
		set __demir_intrin = 1002

	proc/Read(savefile/F)
		set __demir_intrin = 1003

	proc/Write(savefile/F)
		set __demir_intrin = 1004

///
/// LISTS
///

/list
	var/len
	var/const/type = /list

	proc/New(Size)
	proc/Add(Item1)
		set __demir_intrin = 1100
	proc/Copy(Start = 1, End = 0)
		set __demir_intrin = 1101
	proc/Cut(Start = 1, End = 0)
		set __demir_intrin = 1102
	proc/Find(Elem, Start = 1, End = 0)
		set __demir_intrin = 1103
	proc/Insert(Index, Item1)
		set __demir_intrin = 1104
	proc/Join(Glue, Start = 1, End = 0)
		set __demir_intrin = 1105
	proc/Remove(Item1)
		set __demir_intrin = 1106
	proc/RemoveAll(Item1)
		set __demir_intrin = 1107
	proc/Swap(Index1, Index2)
		set __demir_intrin = 1108
	proc/Splice(Start = 1, End = 0, Item1, ...)
		set __demir_intrin = 1109

/alist
	parent_type = /list
	var/const/type = /alist
	proc/New(items)
