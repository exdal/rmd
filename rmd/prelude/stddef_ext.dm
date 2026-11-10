// Next available intrinsic indices by range:
// World: 123, global procedures: 404.
// Native hooks: icon 614, database 621, generator 631.
// Types: atom/image 1026, movable 1032, mob 1042, client 1077,
//        savefile 1086, geometry 1097.
// PLEASE UPDATE THIS WHEN YOU ADD NERW ID!!!

// Atom types

/atom
	parent_type = /datum

	var/name = null
	var/text = null
	var/desc = null
	var/suffix = null

	var/list/verbs = null
	var/list/contents = null
	var/list/overlays = null
	var/list/underlays = null
	var/list/vis_locs = null
	var/list/vis_contents = null

	var/atom/loc
	var/dir = SOUTH
	var/x = 0
	var/y = 0
	var/z = 0
	var/pixel_x = 0
	var/pixel_y = 0
	var/pixel_z = 0
	var/pixel_w = 0

	var/icon_w = 0
	var/icon_z = 0

	var/icon = null as icon|null
	var/icon_state = null as text|null
	var/layer = 2.0
	var/plane = 0
	var/alpha = 255
	var/color = null as /list|text|null
	var/invisibility = 0
	var/mouse_opacity = 1
	var/infra_luminosity = 0
	var/luminosity = 0
	var/opacity = FALSE
	var/matrix/transform = matrix()
	var/blend_mode = 0

	var/gender = NEUTER
	var/density = FALSE

	var/maptext = null

	var/list/filters = null
	var/appearance
	var/appearance_flags = 0
	var/maptext_width = 32
	var/maptext_height = 32
	var/maptext_x = 0
	var/maptext_y = 0
	var/step_x = 0
	var/step_y = 0
	var/render_source
	var/mouse_drag_pointer
	var/mouse_drop_pointer = MOUSE_ACTIVE_POINTER
	var/mouse_over_pointer
	var/mouse_drop_zone = FALSE
	var/render_target
	var/vis_flags
	var/pixloc/pixloc

	proc/Click(location, control, params)
		set __demir_intrin = "AtomClick"

	proc/DblClick(location, control, params)
		set __demir_intrin = "AtomDblClick"

	proc/MouseDown(location, control, params)
		set __demir_intrin = "AtomMouseDown"

	proc/MouseDrag(over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = "AtomMouseDrag"

	proc/MouseDrop(over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = "AtomMouseDrop"

	proc/MouseEntered(location,control,params)
		set __demir_intrin = "AtomMouseEntered"

	proc/MouseExited(location,control,params)
		set __demir_intrin = "AtomMouseExited"

	proc/MouseMove(location,control,params)
		set __demir_intrin = "AtomMouseMove"

	proc/MouseUp(location,control,params)
		set __demir_intrin = "AtomMouseUp"

	proc/MouseWheel(delta_x,delta_y,location,control,params)
		set __demir_intrin = "AtomMouseWheel"

	proc/Entered(atom/movable/Obj, atom/OldLoc)
		set __demir_intrin = "AtomEntered"

	proc/Exited(atom/movable/Obj, atom/newloc)
		set __demir_intrin = "AtomExited"

	proc/Crossed(atom/movable/O)
		set __demir_intrin = "AtomCrossed"

	proc/Uncrossed(atom/movable/O)
		set __demir_intrin = "AtomUncrossed"

	proc/Stat()
		set __demir_intrin = "AtomStat"

	proc/Cross(atom/movable/O)
		return !(src.density && O.density)

	proc/Uncross(atom/movable/O)
		return TRUE

	proc/Enter(atom/movable/O, atom/oldloc)
		return TRUE

	proc/Exit(atom/movable/O, atom/newloc)
		return TRUE

	New(loc)
		..()

/atom/movable
	var/screen_loc
	var/animate_movement = FORWARD_STEPS
	var/list/locs = null
	var/glide_size = 0
	var/step_size
	var/bound_x
	var/bound_y
	var/bound_width
	var/bound_height
	var/bounds
	var/particles/particles

	proc/Bump(atom/Obstacle)
		set __demir_intrin = "MovableBump"

	proc/Move(atom/NewLoc, Dir=0)
		set __demir_intrin = "MovableMove"

/area
	parent_type = /atom
	layer = AREA_LAYER
	luminosity = 1

/turf
	parent_type = /atom
	layer = TURF_LAYER

/obj
	parent_type = /atom/movable
	layer = OBJ_LAYER

/mob
	parent_type = /atom/movable
	layer = MOB_LAYER
	var/client/client
	var/key as text|null
	var/ckey as text|null
	var/list/group
	var/see_invisible = 0
	var/see_infrared = 0
	var/sight = 0
	var/see_in_dark = 2
	density = TRUE

	proc/Login()
		set __demir_intrin = "MobLogin"

	proc/Logout()
		set __demir_intrin = "MobLogout"

// Appearance holders

/image
	parent_type = /datum
	var/alpha = 255
	var/appearance
	var/appearance_flags = 0
	var/blend_mode = 0
	var/color = null
	var/list/contents
	var/density = FALSE
	var/desc = null
	var/gender = NEUTER
	var/glide_size = 0
	var/infra_luminosity = 0
	var/invisibility
	var/list/filters = list()
	var/layer = FLOAT_LAYER
	var/luminosity = 0
	var/maptext = null
	var/maptext_width = 32
	var/maptext_height = 32
	var/maptext_x = 0
	var/maptext_y = 0
	var/mouse_over_pointer = 0
	var/mouse_drag_pointer = 0
	var/mouse_drop_pointer = 1
	var/mouse_drop_zone = FALSE
	var/mouse_opacity = 1
	var/name = "image"
	var/opacity = FALSE
	var/list/overlays = null
	var/override = 0
	var/pixel_step_size = 0
	var/pixel_x = 0
	var/pixel_y = 0
	var/pixel_w = 0
	var/pixel_z = 0
	var/plane = FLOAT_PLANE
	var/render_source
	var/render_target
	var/suffix
	var/text = "i"
	var/matrix/transform = matrix()
	var/list/underlays = null
	var/list/verbs
	var/visibility = 1
	var/vis_flags = 0
	var/bound_width
	var/bound_height
	var/x
	var/y
	var/z
	var/list/vis_contents
	var/dir
	var/icon
	var/icon_state
	var/atom/loc

	New(icon, loc, icon_state, layer, dir, pixel_x, pixel_y)
		set __demir_intrin = "ImageNew"

/mutable_appearance
	parent_type = /image
	var/animate_movement = 1
	var/screen_loc
// Client

/client
	var/list/verbs = null
	var/list/screen = null
	var/list/images = null
	var/list/vars

	var/atom/statobj
	var/statpanel
	var/default_verb_category = "Commands"

	var/tag
	var/const/type = /client

	var/mob/mob
	var/atom/eye
	var/lazy_eye = 0
	var/perspective = MOB_PERSPECTIVE
	var/edge_limit = null
	var/view
	var/pixel_x = 0
	var/pixel_y = 0
	var/pixel_z = 0
	var/pixel_w = 0
	var/show_popup_menus = 1
	var/show_verb_panel = 1

	var/byond_version = 516
	var/byond_build = 1665

	var/address
	var/inactivity = 0
	var/key as text|null
	var/ckey as text|null
	var/connection
	var/computer_id = 0
	var/tick_lag = 0
	var/authenticate = TRUE

	var/timezone

	var/script
	var/color = 0
	var/control_freak
	var/mouse_pointer_icon
	var/preload_rsc = 1
	var/fps = 0
	var/dir = NORTH
	var/gender = NEUTER
	var/glide_size
	var/virtual_eye

	var/list/bounds
	var/bound_x
	var/bound_y
	var/bound_width
	var/bound_height

	proc/New(TopicData)
		set __demir_intrin = "ClientNew"

	proc/Del()
		set __demir_intrin = "ClientDel"

	proc/Topic(href, list/href_list, datum/hsrc)
		set __demir_intrin = "ClientTopic"

	proc/Stat()
		set __demir_intrin = "ClientStat"

	proc/Command(command)
		set __demir_intrin = "ClientCommand"

	proc/Import(Query)
		set __demir_intrin = "ClientImport"

	proc/Export(file)
		set __demir_intrin = "ClientExport"

	proc/AllowUpload(filename, filelength)
		set __demir_intrin = "ClientAllowUpload"

	proc/SoundQuery()
		set __demir_intrin = "ClientSoundQuery"

	proc/MeasureText(text, style, width=0)
		set __demir_intrin = "ClientMeasureText"

	proc/Move(loc, dir)
		set __demir_intrin = "ClientMove"

	proc/Click(atom/object, location, control, params)
		set __demir_intrin = "ClientClick"

	proc/DblClick(atom/object, location, control, params)
		set __demir_intrin = "ClientDblClick"

	proc/MouseDown(atom/object, location, control, params)
		set __demir_intrin = "ClientMouseDown"

	proc/MouseDrag(atom/src_object,over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = "ClientMouseDrag"

	proc/MouseDrop(atom/src_object,over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = "ClientMouseDrop"

	proc/MouseEntered(atom/object,location,control,params)
		set __demir_intrin = "ClientMouseEntered"

	proc/MouseExited(atom/object,location,control,params)
		set __demir_intrin = "ClientMouseExited"

	proc/MouseMove(atom/object,location,control,params)
		set __demir_intrin = "ClientMouseMove"

	proc/MouseUp(atom/object,location,control,params)
		set __demir_intrin = "ClientMouseUp"

	proc/MouseWheel(atom/object,delta_x,delta_y,location,control,params)
		set __demir_intrin = "ClientMouseWheel"

	proc/IsByondMember()
		set __demir_intrin = "ClientIsByondMember"

	proc/CheckPassport(passport_identifier)
		set __demir_intrin = "ClientCheckPassport"

	proc/SendPage(msg, recipient, options)
		set __demir_intrin = "ClientSendPage"

	proc/GetAPI(Api, Name)
		set __demir_intrin = "ClientGetAPI"

	proc/SetAPI(Api, Key, Value)
		set __demir_intrin = "ClientSetAPI"

	proc/RenderIcon(object)
		set __demir_intrin = "ClientRenderIcon"

// Savefiles

/savefile
	var/cd
	var/list/dir
	var/eof
	var/name

	proc/New(filename, timeout)
		set __demir_intrin = "SavefileNew"

	proc/Flush()
		set __demir_intrin = "SavefileFlush"

	proc/ExportText(path = cd, file)
		set __demir_intrin = "SavefileExportText"

	proc/ImportText(path = cd, source)
		set __demir_intrin = "SavefileImportText"

	proc/Lock(timeout)
		set __demir_intrin = "SavefileLock"

	proc/Unlock()
		set __demir_intrin = "SavefileUnlock"

// Geometry

/vector
	var/len
	var/size
	var/x = 0
	var/y = 0
	var/z = 0

	proc/New(x, y, z)
		set __demir_intrin = "VectorNew"

	proc/Cross(vector/B)
		set __demir_intrin = "VectorCross"

	proc/Dot(vector/B)
		set __demir_intrin = "VectorDot"

	proc/Interpolate(vector/B, t)
		set __demir_intrin = "VectorInterpolate"

	proc/Normalize()
		set __demir_intrin = "VectorNormalize"

	proc/Turn(angle)
		set __demir_intrin = "VectorTurn"

/pixloc
	var/turf/loc
	var/step_x
	var/step_y
	var/x
	var/y
	var/z

	proc/New(x, y, z)
		set __demir_intrin = "PixlocNew"

// World

/world
	var/const/type
	var/const/parent_type
	var/const/list/vars
	var/tag

	var/name = "byond"
	var/list/contents
	var/log

	var/area/area = /area
	var/turf/turf = /turf
	var/mob/mob = /mob

	var/time
	var/timeofday
	var/timezone
	var/realtime
	var/tick_lag = 1
	var/tick_usage
	var/fps = 10
	var/cpu
	var/map_cpu
	var/loop_checks = 1

	var/maxx
	var/maxy
	var/maxz
	var/icon_size = 32
	var/view = 5
	var/map_format = TOPDOWN_MAP
	var/movement_mode = LEGACY_MOVEMENT_MODE

	var/byond_version = 516
	var/byond_build = 1665
	var/version = 0
	var/system_type

	var/address
	var/port
	var/internet_address
	var/url
	var/list/params
	var/process
	var/executor
	var/reachable

	var/visibility = 1
	var/status
	var/sleep_offline = 0
	var/cache_lifespan = 30

	var/hub
	var/hub_password
	var/host
	var/game_state = 0

	proc/New()
		set __demir_intrin = "WorldNew"
	proc/Del()
		set __demir_intrin = "WorldDel"
	proc/Topic(T, Addr, Master, Keys)
		set __demir_intrin = "WorldTopic"
	proc/Error(exception)
		set __demir_intrin = "WorldError"

	proc/Reboot(reason)
		set __demir_intrin = "WorldReboot"
	proc/Repop()
		set __demir_intrin = "WorldRepop"

	proc/Export(Addr, File, Persist, Clients)
		set __demir_intrin = "WorldExport"
	proc/Import()
		set __demir_intrin = "WorldImport"
	proc/OpenPort(port)
		set __demir_intrin = "WorldOpenPort"
	proc/Profile(command, type, format)
		set __demir_intrin = "WorldProfile"

	proc/GetConfig(config_set, param)
		set __demir_intrin = "WorldGetConfig"
	proc/SetConfig(config_set, param, value)
		set __demir_intrin = "WorldSetConfig"

	proc/IsBanned(key, address, computer_id, type)
		set __demir_intrin = "WorldIsBanned"
	proc/IsSubscribed(player, type)
		set __demir_intrin = "WorldIsSubscribed"

	proc/AddCredits(player, credits, note)
		set __demir_intrin = "WorldAddCredits"
	proc/GetCredits(player)
		set __demir_intrin = "WorldGetCredits"
	proc/PayCredits(player, credits, note)
		set __demir_intrin = "WorldPayCredits"
	proc/GetScores(key, fields, count, skip)
		set __demir_intrin = "WorldGetScores"
	proc/SetScores(key, fields)
		set __demir_intrin = "WorldSetScores"
	proc/GetMedal(medal, player)
		set __demir_intrin = "WorldGetMedal"
	proc/SetMedal(medal, player)
		set __demir_intrin = "WorldSetMedal"
	proc/ClearMedal(medal, player)
		set __demir_intrin = "WorldClearMedal"

	proc/file2list(File, Separator)
		set __demir_intrin = "WorldFile2List"

// Global procedures

/proc/call(Target, ProcName)
	set __demir_intrin = "Call"

/proc/call_ext(Target, ProcName)
	set __demir_intrin = "CallExt"

/proc/arglist(Arguments)
	set __demir_intrin = "Arglist"

/proc/abs(A)
	set __demir_intrin = "Abs"

/proc/addtext(Arg1,Arg2)
	set __demir_intrin = "Addtext"

/proc/alert(Usr,Message,Title,Button1,Button2,Button3)
	set __demir_intrin = "Alert"

/proc/alist(A,B,C)
	set __demir_intrin = "Alist"

/proc/animate(Object,time,loop,easing,flags,delay)
	set __demir_intrin = "Animate"

/proc/arccos(X)
	set __demir_intrin = "Arccos"

/proc/arcsin(X)
	set __demir_intrin = "Arcsin"

/proc/arctan(A,B)
	set __demir_intrin = "Arctan"

/proc/ascii2text(N)
	set __demir_intrin = "Ascii2text"

/proc/astype(Val,Type)
	set __demir_intrin = "Astype"

/proc/block(Start,End)
	set __demir_intrin = "Block"

/proc/bound_pixloc(Atom,Dir)
	set __demir_intrin = "BoundPixloc"

/proc/bounds_dist(Ref,Target)
	set __demir_intrin = "BoundsDist"

/proc/bounds(Ref,Dist)
	set __demir_intrin = "Bounds"

/proc/browse(Body,Options)
	set __demir_intrin = "Browse"

/proc/browse_rsc(File,FileName)
	set __demir_intrin = "BrowseRsc"

/proc/ceil(A)
	set __demir_intrin = "Ceil"

/proc/ckeyEx(Text)
	set __demir_intrin = "CkeyEx"

/proc/ckey(Key)
	set __demir_intrin = "Ckey"

/proc/clamp(NumberOrList,Low,High)
	set __demir_intrin = "Clamp"

/proc/cmptextEx(T1,T2)
	set __demir_intrin = "CmptextEx"

/proc/cmptext(T1,T2)
	set __demir_intrin = "Cmptext"

/proc/copytext_char(T,Start,End)
	set __demir_intrin = "CopytextChar"

/proc/copytext(T,Start,End)
	set __demir_intrin = "Copytext"

/proc/cos(X)
	set __demir_intrin = "Cos"

/proc/CRASH(message)
	set __demir_intrin = "Crash"

/proc/_dm_db_close(db_query)
	set __demir_intrin = "DmDbClose"

/proc/_dm_db_columns(db_query,db_column)
	set __demir_intrin = "DmDbColumns"

/proc/_dm_db_connect(db_con,dbi_handler,user_handler,password_handler,cursor_handler,unknown)
	set __demir_intrin = "DmDbConnect"

/proc/_dm_db_error_msg(db_query)
	set __demir_intrin = "DmDbErrorMsg"

/proc/_dm_db_execute(db_query,sql_query,db_connection,cursor_handler,unknown)
	set __demir_intrin = "DmDbExecute"

/proc/_dm_db_is_connected(db_con)
	set __demir_intrin = "DmDbIsConnected"

/proc/_dm_db_new_con()
	set __demir_intrin = "DmDbNewCon"

/proc/_dm_db_new_query()
	set __demir_intrin = "DmDbNewQuery"

/proc/_dm_db_next_row(db_query,item,conversions)
	set __demir_intrin = "DmDbNextRow"

/proc/_dm_db_quote(db_con,_str)
	set __demir_intrin = "DmDbQuote"

/proc/_dm_db_row_count(db_query)
	set __demir_intrin = "DmDbRowCount"

/proc/_dm_db_rows_affected(db_query)
	set __demir_intrin = "DmDbRowsAffected"

/proc/fcopy_rsc(File)
	set __demir_intrin = "FcopyRsc"

/proc/fcopy(Src,Dst)
	set __demir_intrin = "Fcopy"

/proc/fdel(File)
	set __demir_intrin = "Fdel"

/proc/fexists(File)
	set __demir_intrin = "Fexists"

/proc/file2text(File)
	set __demir_intrin = "File2text"

/proc/file(Path)
	set __demir_intrin = "File"

/proc/filter(type)
	set __demir_intrin = "Filter"

/proc/findlasttext_char(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = "FindlasttextChar"

/proc/findlasttextEx_char(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = "FindlasttextExChar"

/proc/findlasttextEx(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = "FindlasttextEx"

/proc/findlasttext(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = "Findlasttext"

/proc/findtext_char(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = "FindtextChar"

/proc/findtextEx_char(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = "FindtextExChar"

/proc/findtextEx(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = "FindtextEx"

/proc/findtext(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = "Findtext"

/proc/flick(Icon,Object)
	set __demir_intrin = "Flick"

/proc/flist(Path)
	set __demir_intrin = "Flist"

/proc/floor(A)
	set __demir_intrin = "Floor"

/proc/fract(A)
	set __demir_intrin = "Fract"

/proc/ftime(File,IsCreationTime)
	set __demir_intrin = "Ftime"

/proc/ftp(File,Name)
	set __demir_intrin = "Ftp"

/proc/get_dir(Loc1,Loc2)
	set __demir_intrin = "GetDir"

/proc/get_dist(Loc1,Loc2)
	set __demir_intrin = "GetDist"

/proc/get_step_away(Ref,Trg,Max=5)
	set __demir_intrin = "GetStepAway"

/proc/get_step_rand(Ref)
	set __demir_intrin = "GetStepRand"

/proc/get_step(Ref,Dir)
	set __demir_intrin = "GetStep"

/proc/get_steps_to(Ref,Trg,Min=0)
	set __demir_intrin = "GetStepsTo"

/proc/get_step_to(Ref,Trg,Min=0)
	set __demir_intrin = "GetStepTo"

/proc/get_step_towards(Ref,Trg)
	set __demir_intrin = "GetStepTowards"

/proc/generator(type,A,B,rand)
	set __demir_intrin = "Generator"

/proc/gradient(Gradient,index,space=COLORSPACE_RGB)
	set __demir_intrin = "Gradient"

/proc/hascall(Object,ProcName)
	set __demir_intrin = "Hascall"

/proc/hearers(Depth=world.view,Center=usr)
	set __demir_intrin = "Hearers"

/proc/html_decode(HtmlText)
	set __demir_intrin = "HtmlDecode"

/proc/html_encode(PlainText)
	set __demir_intrin = "HtmlEncode"

/proc/icon(icon,icon_state,dir,frame,moving)
	set __demir_intrin = "Icon"

/proc/icon_states(Icon,mode=0)
	set __demir_intrin = "IconStates"

/proc/image(icon,loc,icon_state,layer,dir,pixel_x,pixel_y,pixel_w,pixel_z)
	set __demir_intrin = "Image"

/proc/isarea(Loc1,Loc2)
	set __demir_intrin = "Isarea"

/proc/isfile(File)
	set __demir_intrin = "Isfile"

/proc/isicon(Icon)
	set __demir_intrin = "Isicon"

/proc/isinf(A)
	set __demir_intrin = "Isinf"

/proc/islist(List)
	set __demir_intrin = "Islist"

/proc/isloc(Loc1,Loc2)
	set __demir_intrin = "Isloc"

/proc/ismob(Loc1,Loc2)
	set __demir_intrin = "Ismob"

/proc/ismovable(Loc1,Loc2)
	set __demir_intrin = "Ismovable"

/proc/isnan(A)
	set __demir_intrin = "Isnan"

/proc/isnull(Val)
	set __demir_intrin = "Isnull"

/proc/isnum(Val)
	set __demir_intrin = "Isnum"

/proc/isobj(Loc1,Loc2)
	set __demir_intrin = "Isobj"

/proc/ispath(Val,Type)
	set __demir_intrin = "Ispath"

/proc/ispointer(Value)
	set __demir_intrin = "Ispointer"

/proc/issaved(Var)
	set __demir_intrin = "Issaved"

/proc/istext(Val)
	set __demir_intrin = "Istext"

/proc/isturf(Loc1,Loc2)
	set __demir_intrin = "Isturf"

/proc/istype(Val,Type)
	set __demir_intrin = "Istype"

/proc/jointext(List,Glue,Start=1,End=0)
	set __demir_intrin = "Jointext"

/proc/json_decode(JSON)
	set __demir_intrin = "JsonDecode"

/proc/json_encode(Value)
	set __demir_intrin = "JsonEncode"

/proc/length_char(E)
	set __demir_intrin = "LengthChar"

/proc/length(E)
	set __demir_intrin = "Length"

/proc/lentext(T)
	set __demir_intrin = "Lentext"

/proc/lerp(A,B,factor)
	set __demir_intrin = "Lerp"

/proc/link(url)
	set __demir_intrin = "Link"

/proc/list2params(List)
	set __demir_intrin = "List2params"

/proc/load_ext(LibName,FuncName)
	set __demir_intrin = "LoadExt"

/proc/load_resource(File,KeepTime)
	set __demir_intrin = "LoadResource"

/proc/locate(Type)
	set __demir_intrin = "Locate"

/proc/log(X=2.718,Y)
	set __demir_intrin = "Log"

/proc/lowertext(T)
	set __demir_intrin = "Lowertext"

/proc/matrix(a,b,c,d,e,f)
	set __demir_intrin = "Matrix"

/proc/max(A,B,C)
	set __demir_intrin = "Max"

/proc/md5(T)
	set __demir_intrin = "Md5"

/proc/min(A,B,C)
	set __demir_intrin = "Min"

/proc/missile(Type,Start,End)
	set __demir_intrin = "Missile"

/proc/mutable_appearance(appearance,key)
	set __demir_intrin = "MutableAppearance"

/proc/newlist(A,B,C)
	set __demir_intrin = "Newlist"

/proc/noise_hash(param1)
	set __demir_intrin = "NoiseHash"

/proc/nonspantext_char(Haystack,Needles,Start=1)
	set __demir_intrin = "NonspantextChar"

/proc/nameof(X)
	set __demir_intrin = "Nameof"

/proc/nonspantext(Haystack,Needles,Start=1)
	set __demir_intrin = "Nonspantext"

/proc/num2text(N,SigFig=6,Radix)
	set __demir_intrin = "Num2text"

/proc/obounds(Ref=src,Dist=0)
	set __demir_intrin = "Obounds"

/proc/ohearers(Depth=world.view,Center=usr)
	set __demir_intrin = "Ohearers"

/proc/orange(Dist,Center=usr)
	set __demir_intrin = "Orange"

/proc/output(msg,control)
	set __demir_intrin = "Output"

/proc/oview(Dist,Center=usr)
	set __demir_intrin = "Oview"

/proc/oviewers(Depth=world.view,Center=usr)
	set __demir_intrin = "Oviewers"

/proc/params2list(Params)
	set __demir_intrin = "Params2list"

/proc/pixloc(x,y,z)
	set __demir_intrin = "Pixloc"

/proc/prob(P)
	set __demir_intrin = "Prob"

/proc/rand(L=0,H)
	set __demir_intrin = "Rand"

/proc/rand_seed(Seed)
	set __demir_intrin = "RandSeed"

/proc/range(Dist,Center=usr)
	set __demir_intrin = "Range"

/proc/ref(A)
	set __demir_intrin = "Ref"

/proc/refcount(Object)
	set __demir_intrin = "Refcount"

/proc/regex(pattern,flags)
	set __demir_intrin = "Regex"

/proc/replacetext_char(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = "ReplacetextChar"

/proc/replacetextEx_char(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = "ReplacetextExChar"

/proc/replacetextEx(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = "ReplacetextEx"

/proc/replacetext(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = "Replacetext"

/proc/rgb2num(color,space)
	set __demir_intrin = "Rgb2num"

/proc/rgb(R,G,B,A=null,space,red,blue,green,alpha,h,hue,s,saturation,c,chroma,v,value,l,y,luminance)
	set __demir_intrin = "Rgb"

/proc/roll(ndice=1,sides)
	set __demir_intrin = "Roll"

/proc/round(A,B=null)
	set __demir_intrin = "Round"

/proc/run(File)
	set __demir_intrin = "Run"

/proc/sha1(StringOrFile)
	set __demir_intrin = "Sha1"

/proc/shell(Command)
	set __demir_intrin = "Shell"

/proc/shutdown(Addr,Natural=0)
	set __demir_intrin = "Shutdown"

/proc/sign(A)
	set __demir_intrin = "Sign"

/proc/sin(X)
	set __demir_intrin = "Sin"

/proc/sleep(Delay)
	set __demir_intrin = "Sleep"

/proc/sorttextEx(T1,T2)
	set __demir_intrin = "SorttextEx"

/proc/sorttext(T1,T2)
	set __demir_intrin = "Sorttext"

/proc/sound(file,repeat=0,wait,channel,volume)
	set __demir_intrin = "Sound"

/proc/spantext_char(Haystack,Needles,Start=1)
	set __demir_intrin = "SpantextChar"

/proc/spantext(Haystack,Needles,Start=1)
	set __demir_intrin = "Spantext"

/proc/splicetext_char(Text,Start=1,End=0,Insert="")
	set __demir_intrin = "SplicetextChar"

/proc/splicetext(Text,Start=1,End=0,Insert="")
	set __demir_intrin = "Splicetext"

/proc/splittext_char(Text,Delimiter,Start=1,End=0,include_delimiters=0)
	set __demir_intrin = "SplittextChar"

/proc/splittext(Text,Delimiter,Start=1,End=0,include_delimiters=0)
	set __demir_intrin = "Splittext"

/proc/sqrt(A)
	set __demir_intrin = "Sqrt"

/proc/startup(File,Port=0,Options)
	set __demir_intrin = "Startup"

/proc/stat(Name,Value)
	set __demir_intrin = "Stat"

/proc/statpanel(Panel,Name,Value)
	set __demir_intrin = "Statpanel"

/proc/step_away(Ref,Trg,Max=5,Speed=0)
	set __demir_intrin = "StepAway"

/proc/step_rand(Ref,Speed=0)
	set __demir_intrin = "StepRand"

/proc/step(Ref,Dir,Speed=0)
	set __demir_intrin = "Step"

/proc/step_to(Ref,Trg,Min=0,Speed=0)
	set __demir_intrin = "StepTo"

/proc/step_towards(Ref,Trg,Speed)
	set __demir_intrin = "StepTowards"

/proc/tan(A)
	set __demir_intrin = "Tan"

/proc/text2ascii_char(T,pos=1)
	set __demir_intrin = "Text2asciiChar"

/proc/text2ascii(T,pos=1)
	set __demir_intrin = "Text2ascii"

/proc/text2file(Text,File)
	set __demir_intrin = "Text2file"

/proc/text2num(T,Radix)
	set __demir_intrin = "Text2num"

/proc/text2path(T)
	set __demir_intrin = "Text2path"

/proc/text(FormatText,Args)
	set __demir_intrin = "Text"

/proc/time2text(timestamp,format,timezone)
	set __demir_intrin = "Time2text"

/proc/trimtext(Text)
	set __demir_intrin = "Trimtext"

/proc/trunc(A)
	set __demir_intrin = "Trunc"

/proc/turn(Dir,Angle)
	set __demir_intrin = "Turn"

/proc/typesof(Type1,Type2)
	set __demir_intrin = "Typesof"

/proc/uppertext(T)
	set __demir_intrin = "Uppertext"

/proc/url_decode(UrlText)
	set __demir_intrin = "UrlDecode"

/proc/url_encode(PlainText,format=0)
	set __demir_intrin = "UrlEncode"

/proc/values_cut_over(Alist,Max,inclusive=0)
	set __demir_intrin = "ValuesCutOver"

/proc/values_cut_under(Alist,Max,inclusive=0)
	set __demir_intrin = "ValuesCutUnder"

/proc/values_dot(A,B)
	set __demir_intrin = "ValuesDot"

/proc/values_product(Alist)
	set __demir_intrin = "ValuesProduct"

/proc/values_sum(Alist)
	set __demir_intrin = "ValuesSum"

/proc/vector(x,y,z)
	set __demir_intrin = "Vector"

/proc/view(Dist=5,Center=usr)
	set __demir_intrin = "View"

/proc/viewers(Depth=world.view,Center=usr)
	set __demir_intrin = "Viewers"

/proc/walk_away(Ref,Trg,Max=5,Lag=0,Speed=0)
	set __demir_intrin = "WalkAway"

/proc/walk_rand(Ref,Lag=0,Speed=0)
	set __demir_intrin = "WalkRand"

/proc/walk(Ref,Dir,Lag=0,Speed=0)
	set __demir_intrin = "Walk"

/proc/walk_to(Ref,Trg,Min=0,Lag=0,Speed=0)
	set __demir_intrin = "WalkTo"

/proc/walk_towards(Ref,Trg,Lag=0,Speed=0)
	set __demir_intrin = "WalkTowards"

/proc/winclone(player,window_name,clone_name)
	set __demir_intrin = "Winclone"

/proc/winexists(player,control_id)
	set __demir_intrin = "Winexists"

/proc/winget(player,control_id,params)
	set __demir_intrin = "Winget"

/proc/winset(player,control_id,params)
	set __demir_intrin = "Winset"

/proc/winshow(player,window,show=1)
	set __demir_intrin = "Winshow"

// Native hooks

/proc/_dm_new_icon(icon,icon_state,dir,frame,moving)
	set __demir_intrin = "DmNewIcon"

/proc/_dm_turn_icon(icon,angle,antialias)
	set __demir_intrin = "DmTurnIcon"

/proc/_dm_flip_icon(icon,dir)
	set __demir_intrin = "DmFlipIcon"

/proc/_dm_shift_icon(icon,dir,offset,wrap)
	set __demir_intrin = "DmShiftIcon"

/proc/_dm_icon_intensity(icon,r,g,b)
	set __demir_intrin = "DmIconIntensity"

/proc/_dm_icon_blend(icon,other,function,x,y)
	set __demir_intrin = "DmIconBlend"

/proc/_dm_icon_swap_color(icon,old_rgb,new_rgb)
	set __demir_intrin = "DmIconSwapColor"

/proc/_dm_icon_draw_box(icon,rgb,x1,y1,x2,y2)
	set __demir_intrin = "DmIconDrawBox"

/proc/_dm_icon_insert(icon,new_icon,icon_state,dir,frame,moving,delay)
	set __demir_intrin = "DmIconInsert"

/proc/_dm_icon_map_colors(icon,...)
	set __demir_intrin = "DmIconMapColors"

/proc/_dm_icon_scale(icon,width,height)
	set __demir_intrin = "DmIconScale"

/proc/_dm_icon_crop(icon,x1,y1,x2,y2)
	set __demir_intrin = "DmIconCrop"

/proc/_dm_icon_getpixel(icon,x,y,icon_state,dir,frame,moving)
	set __demir_intrin = "DmIconGetPixel"

/proc/_dm_icon_size(icon,axis)
	set __demir_intrin = "DmIconSize"

/proc/_dm_database(db,operation,argument)
	set __demir_intrin = "DmDatabase"

/proc/make_generator(type,rand,a,b)
	set __demir_intrin = "MakeGenerator"
