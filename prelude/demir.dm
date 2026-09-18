#define __DEMIR__

#ifndef __DEMIR_BAKE__
#define __DEMIR_COMPAT__
#endif
#ifdef __DEMIR_COMPAT__
// mapping tools set this, and codebases gate map editor icon states on it
#define FASTDMM  // we do a little bit of lying
#define SPACEMAN_DMM
#define SpacemanDMM_unlint(X) X
#define SpacemanDMM_debug(X...) X
#endif

///
/// ATOM AND ITS DERIVATIVES
///

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
	var/opacity = 0
	var/matrix/transform
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

	var/demir_light_range = 0
	/// Radius of full brightness inside `demir_light_range`.
	var/demir_light_inner_range = 0
	var/demir_light_power = 0
	var/demir_light_color = null
	var/demir_light_angle = 360
	var/demir_light_dir = 0
	/// Additional light-source origin offset in tile units.
	var/demir_light_offset_x = 0
	var/demir_light_offset_y = 0
	var/demir_light_height = 1
	/// Falloff exponent.
	var/demir_light_curve = 1
	/// Combine with other peak sources by taking the strongest instead of summing.
	var/demir_light_peak = 0
	/// Only contribute when this cell touches one that is not fullbright.
	var/demir_light_edge_only = 0
	/// Use `demir_light_quadratic / distance ** 2` when nonzero.
	var/demir_light_quadratic = 0
	/// Constant added to the quadratic term.
	var/demir_light_constant = 0
	/// A value of -1 follows `opacity`.
	var/demir_blocks_light = -1
	var/demir_ambient_color = null
	/// Ambient power in the range 0..1.
	var/demir_ambient_power = 0
	/// Suppress corner lighting for this cell.
	var/demir_fullbright = 0
	/// Use this appearance as an emissive mask without drawing it into the scene color.
	var/demir_emissive = 0
	/// Clear emissive pixels behind this appearance without drawing it.
	var/demir_emissive_blocker = 0
	/// Add (positive) or subtract (negative) this appearance from the cheap overlay lightmap.
	var/demir_overlay_light = 0

	proc/Click(location, control, params)
		set __demir_intrin = 1010

	proc/DblClick(location, control, params)
		set __demir_intrin = 1011

	proc/MouseDown(location, control, params)
		set __demir_intrin = 1012

	proc/MouseDrag(over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = 1013

	proc/MouseDrop(over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = 1014

	proc/MouseEntered(location,control,params)
		set __demir_intrin = 1015

	proc/MouseExited(location,control,params)
		set __demir_intrin = 1016

	proc/MouseMove(location,control,params)
		set __demir_intrin = 1017

	proc/MouseUp(location,control,params)
		set __demir_intrin = 1018

	proc/MouseWheel(delta_x,delta_y,location,control,params)
		set __demir_intrin = 1019

	proc/Entered(atom/movable/Obj, atom/OldLoc)
		set __demir_intrin = 1020

	proc/Exited(atom/movable/Obj, atom/newloc)
		set __demir_intrin = 1021

	proc/Crossed(atom/movable/O)
		set __demir_intrin = 1022

	proc/Uncrossed(atom/movable/O)
		set __demir_intrin = 1023

	proc/Stat()
		set __demir_intrin = 1024

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
		set __demir_intrin = 1030

	proc/Move(atom/NewLoc, Dir=0)
		set __demir_intrin = 1031

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
		set __demir_intrin = 1040

	proc/Logout()
		set __demir_intrin = 1041

///
/// APPEARANCE HOLDERS
///

/image
	parent_type = /datum
	var/alpha = 255
	var/appearance
	var/appearance_flags = 0
	var/blend_mode = 0
	var/color = null
	var/list/contents
	var/density = 0
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
	var/mouse_drop_zone = 0
	var/mouse_opacity = 1
	var/name = "image"
	var/opacity = 0
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
	var/matrix/transform
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
	var/demir_emissive = 0
	var/demir_emissive_blocker = 0
	var/demir_overlay_light = 0

	New(icon, loc, icon_state, layer, dir, pixel_x, pixel_y)
		set __demir_intrin = 1025

/mutable_appearance
	parent_type = /image
	var/animate_movement = 1
	var/screen_loc

///
/// BAKING HOOKS
///

#define DEMIR_CONNECTION_SOURCE 1
#define DEMIR_CONNECTION_TARGET 2

/proc/demir_bake(atom/target)

/proc/demir_initialize()

/proc/demir_prepare(atom/target)

/proc/demir_light(atom/target)

// Return an associative list of opaque text channel keys to DEMIR_CONNECTION_* role bitmasks.
/proc/demir_connections(atom/target)

#define USE_PERSPECTIVE_EDITOR_WALLS

///
/// CLIENT
///

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
		set __demir_intrin = 1050

	proc/Del()
		set __demir_intrin = 1051

	proc/Topic(href, list/href_list, datum/hsrc)
		set __demir_intrin = 1052

	proc/Stat()
		set __demir_intrin = 1053

	proc/Command(command)
		set __demir_intrin = 1054

	proc/Import(Query)
		set __demir_intrin = 1055

	proc/Export(file)
		set __demir_intrin = 1056

	proc/AllowUpload(filename, filelength)
		set __demir_intrin = 1057

	proc/SoundQuery()
		set __demir_intrin = 1058

	proc/MeasureText(text, style, width=0)
		set __demir_intrin = 1059

	proc/Move(loc, dir)
		set __demir_intrin = 1060

	proc/Click(atom/object, location, control, params)
		set __demir_intrin = 1061

	proc/DblClick(atom/object, location, control, params)
		set __demir_intrin = 1062

	proc/MouseDown(atom/object, location, control, params)
		set __demir_intrin = 1063

	proc/MouseDrag(atom/src_object,over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = 1064

	proc/MouseDrop(atom/src_object,over_object,src_location,over_location,src_control,over_control,params)
		set __demir_intrin = 1065

	proc/MouseEntered(atom/object,location,control,params)
		set __demir_intrin = 1066

	proc/MouseExited(atom/object,location,control,params)
		set __demir_intrin = 1067

	proc/MouseMove(atom/object,location,control,params)
		set __demir_intrin = 1068

	proc/MouseUp(atom/object,location,control,params)
		set __demir_intrin = 1069

	proc/MouseWheel(atom/object,delta_x,delta_y,location,control,params)
		set __demir_intrin = 1070

	proc/IsByondMember()
		set __demir_intrin = 1071

	proc/CheckPassport(passport_identifier)
		set __demir_intrin = 1072

	proc/SendPage(msg, recipient, options)
		set __demir_intrin = 1073

	proc/GetAPI(Api, Name)
		set __demir_intrin = 1074

	proc/SetAPI(Api, Key, Value)
		set __demir_intrin = 1075

	proc/RenderIcon(object)
		set __demir_intrin = 1076

///
/// SAVEFILES
///

/savefile
	var/cd
	var/list/dir
	var/eof
	var/name

	proc/New(filename, timeout)
		set __demir_intrin = 1080

	proc/Flush()
		set __demir_intrin = 1081

	proc/ExportText(path = cd, file)
		set __demir_intrin = 1082

	proc/ImportText(path = cd, source)
		set __demir_intrin = 1083

	proc/Lock(timeout)
		set __demir_intrin = 1084

	proc/Unlock()
		set __demir_intrin = 1085

///
/// GEOMETRY
///

/vector
	var/len
	var/size
	var/x = 0
	var/y = 0
	var/z = 0

	proc/New(x, y, z)
		set __demir_intrin = 1090

	proc/Cross(vector/B)
		set __demir_intrin = 1091

	proc/Dot(vector/B)
		set __demir_intrin = 1092

	proc/Interpolate(vector/B, t)
		set __demir_intrin = 1093

	proc/Normalize()
		set __demir_intrin = 1094

	proc/Turn(angle)
		set __demir_intrin = 1095

/pixloc
	var/turf/loc
	var/step_x
	var/step_y
	var/x
	var/y
	var/z

	proc/New(x, y, z)
		set __demir_intrin = 1096

///
/// WORLD
///

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
		set __demir_intrin = 100
	proc/Del()
		set __demir_intrin = 101
	proc/Topic(T, Addr, Master, Keys)
		set __demir_intrin = 102
	proc/Error(exception)
		set __demir_intrin = 103

	proc/Reboot(reason)
		set __demir_intrin = 104
	proc/Repop()
		set __demir_intrin = 105

	proc/Export(Addr, File, Persist, Clients)
		set __demir_intrin = 106
	proc/Import()
		set __demir_intrin = 107
	proc/OpenPort(port)
		set __demir_intrin = 108
	proc/Profile(command, type, format)
		set __demir_intrin = 109

	proc/GetConfig(config_set, param)
		set __demir_intrin = 110
	proc/SetConfig(config_set, param, value)
		set __demir_intrin = 111

	proc/IsBanned(key, address, computer_id, type)
		set __demir_intrin = 112
	proc/IsSubscribed(player, type)
		set __demir_intrin = 113

	proc/AddCredits(player, credits, note)
		set __demir_intrin = 114
	proc/GetCredits(player)
		set __demir_intrin = 115
	proc/PayCredits(player, credits, note)
		set __demir_intrin = 116
	proc/GetScores(key, fields, count, skip)
		set __demir_intrin = 117
	proc/SetScores(key, fields)
		set __demir_intrin = 118
	proc/GetMedal(medal, player)
		set __demir_intrin = 119
	proc/SetMedal(medal, player)
		set __demir_intrin = 120
	proc/ClearMedal(medal, player)
		set __demir_intrin = 121

	proc/file2list(File, Separator)
		set __demir_intrin = 122

///
/// GLOBAL PROCS
///

/proc/call(Target, ProcName)
	set __demir_intrin = 401

/proc/call_ext(Target, ProcName)
	set __demir_intrin = 402

/proc/arglist(Arguments)
	set __demir_intrin = 403

/proc/abs(A)
	set __demir_intrin = 200

/proc/addtext(Arg1,Arg2)
	set __demir_intrin = 201

/proc/alert(Usr,Message,Title,Button1,Button2,Button3)
	set __demir_intrin = 202

/proc/alist(A,B,C)
	set __demir_intrin = 203

/proc/animate(Object,time,loop,easing,flags,delay)
	set __demir_intrin = 204

/proc/arccos(X)
	set __demir_intrin = 205

/proc/arcsin(X)
	set __demir_intrin = 206

/proc/arctan(A,B)
	set __demir_intrin = 207

/proc/ascii2text(N)
	set __demir_intrin = 208

/proc/astype(Val,Type)
	set __demir_intrin = 209

/proc/block(Start,End)
	set __demir_intrin = 210

/proc/bound_pixloc(Atom,Dir)
	set __demir_intrin = 211

/proc/bounds_dist(Ref,Target)
	set __demir_intrin = 212

/proc/bounds(Ref,Dist)
	set __demir_intrin = 213

/proc/browse(Body,Options)
	set __demir_intrin = 214

/proc/browse_rsc(File,FileName)
	set __demir_intrin = 215

/proc/ceil(A)
	set __demir_intrin = 216

/proc/ckeyEx(Text)
	set __demir_intrin = 217

/proc/ckey(Key)
	set __demir_intrin = 218

/proc/clamp(NumberOrList,Low,High)
	set __demir_intrin = 219

/proc/cmptextEx(T1,T2)
	set __demir_intrin = 220

/proc/cmptext(T1,T2)
	set __demir_intrin = 221

/proc/copytext_char(T,Start,End)
	set __demir_intrin = 222

/proc/copytext(T,Start,End)
	set __demir_intrin = 223

/proc/cos(X)
	set __demir_intrin = 224

/proc/CRASH(message)
	set __demir_intrin = 225

/proc/_dm_db_close(db_query)
	set __demir_intrin = 226

/proc/_dm_db_columns(db_query,db_column)
	set __demir_intrin = 227

/proc/_dm_db_connect(db_con,dbi_handler,user_handler,password_handler,cursor_handler,unknown)
	set __demir_intrin = 228

/proc/_dm_db_error_msg(db_query)
	set __demir_intrin = 229

/proc/_dm_db_execute(db_query,sql_query,db_connection,cursor_handler,unknown)
	set __demir_intrin = 230

/proc/_dm_db_is_connected(db_con)
	set __demir_intrin = 231

/proc/_dm_db_new_con()
	set __demir_intrin = 232

/proc/_dm_db_new_query()
	set __demir_intrin = 233

/proc/_dm_db_next_row(db_query,item,conversions)
	set __demir_intrin = 234

/proc/_dm_db_quote(db_con,_str)
	set __demir_intrin = 235

/proc/_dm_db_row_count(db_query)
	set __demir_intrin = 236

/proc/_dm_db_rows_affected(db_query)
	set __demir_intrin = 237

/proc/fcopy_rsc(File)
	set __demir_intrin = 238

/proc/fcopy(Src,Dst)
	set __demir_intrin = 239

/proc/fdel(File)
	set __demir_intrin = 240

/proc/fexists(File)
	set __demir_intrin = 241

/proc/file2text(File)
	set __demir_intrin = 242

/proc/file(Path)
	set __demir_intrin = 243

/proc/filter(type)
	set __demir_intrin = 244

/proc/findlasttext_char(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = 245

/proc/findlasttextEx_char(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = 246

/proc/findlasttextEx(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = 247

/proc/findlasttext(Haystack,Needle,Start=0,End=1)
	set __demir_intrin = 248

/proc/findtext_char(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = 249

/proc/findtextEx_char(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = 250

/proc/findtextEx(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = 251

/proc/findtext(Haystack,Needle,Start=1,End=0)
	set __demir_intrin = 252

/proc/flick(Icon,Object)
	set __demir_intrin = 253

/proc/flist(Path)
	set __demir_intrin = 254

/proc/floor(A)
	set __demir_intrin = 255

/proc/fract(A)
	set __demir_intrin = 256

/proc/ftime(File,IsCreationTime)
	set __demir_intrin = 257

/proc/ftp(File,Name)
	set __demir_intrin = 258

/proc/get_dir(Loc1,Loc2)
	set __demir_intrin = 259

/proc/get_dist(Loc1,Loc2)
	set __demir_intrin = 260

/proc/get_step_away(Ref,Trg,Max=5)
	set __demir_intrin = 261

/proc/get_step_rand(Ref)
	set __demir_intrin = 262

/proc/get_step(Ref,Dir)
	set __demir_intrin = 263

/proc/get_steps_to(Ref,Trg,Min=0)
	set __demir_intrin = 264

/proc/get_step_to(Ref,Trg,Min=0)
	set __demir_intrin = 265

/proc/get_step_towards(Ref,Trg)
	set __demir_intrin = 266

/proc/generator(type,A,B,rand)
	set __demir_intrin = 399

/proc/gradient(Gradient,index,space=COLORSPACE_RGB)
	set __demir_intrin = 267

/proc/hascall(Object,ProcName)
	set __demir_intrin = 268

/proc/hearers(Depth=world.view,Center=usr)
	set __demir_intrin = 269

/proc/html_decode(HtmlText)
	set __demir_intrin = 270

/proc/html_encode(PlainText)
	set __demir_intrin = 271

/proc/icon(icon,icon_state,dir,frame,moving)
	set __demir_intrin = 272

/proc/icon_states(Icon,mode=0)
	set __demir_intrin = 273

/proc/image(icon,loc,icon_state,layer,dir,pixel_x,pixel_y,pixel_w,pixel_z)
	set __demir_intrin = 274

/proc/isarea(Loc1,Loc2)
	set __demir_intrin = 275

/proc/isfile(File)
	set __demir_intrin = 276

/proc/isicon(Icon)
	set __demir_intrin = 277

/proc/isinf(A)
	set __demir_intrin = 278

/proc/islist(List)
	set __demir_intrin = 279

/proc/isloc(Loc1,Loc2)
	set __demir_intrin = 280

/proc/ismob(Loc1,Loc2)
	set __demir_intrin = 281

/proc/ismovable(Loc1,Loc2)
	set __demir_intrin = 282

/proc/isnan(A)
	set __demir_intrin = 283

/proc/isnull(Val)
	set __demir_intrin = 284

/proc/isnum(Val)
	set __demir_intrin = 285

/proc/isobj(Loc1,Loc2)
	set __demir_intrin = 286

/proc/ispath(Val,Type)
	set __demir_intrin = 287

/proc/ispointer(Value)
	set __demir_intrin = 288

/proc/issaved(Var)
	set __demir_intrin = 289

/proc/istext(Val)
	set __demir_intrin = 290

/proc/isturf(Loc1,Loc2)
	set __demir_intrin = 291

/proc/istype(Val,Type)
	set __demir_intrin = 292

/proc/jointext(List,Glue,Start=1,End=0)
	set __demir_intrin = 293

/proc/json_decode(JSON)
	set __demir_intrin = 294

/proc/json_encode(Value)
	set __demir_intrin = 295

/proc/length_char(E)
	set __demir_intrin = 296

/proc/length(E)
	set __demir_intrin = 297

/proc/lentext(T)
	set __demir_intrin = 298

/proc/lerp(A,B,factor)
	set __demir_intrin = 299

/proc/link(url)
	set __demir_intrin = 300

/proc/list2params(List)
	set __demir_intrin = 301

/proc/load_ext(LibName,FuncName)
	set __demir_intrin = 302

/proc/load_resource(File,KeepTime)
	set __demir_intrin = 303

/proc/locate(Type)
	set __demir_intrin = 304

/proc/log(X=2.718,Y)
	set __demir_intrin = 305

/proc/lowertext(T)
	set __demir_intrin = 306

/proc/matrix(a,b,c,d,e,f)
	set __demir_intrin = 307

/proc/max(A,B,C)
	set __demir_intrin = 308

/proc/md5(T)
	set __demir_intrin = 309

/proc/min(A,B,C)
	set __demir_intrin = 310

/proc/missile(Type,Start,End)
	set __demir_intrin = 311

/proc/mutable_appearance(appearance,key)
	set __demir_intrin = 312

/proc/newlist(A,B,C)
	set __demir_intrin = 313

/proc/noise_hash(param1)
	set __demir_intrin = 314

/proc/nonspantext_char(Haystack,Needles,Start=1)
	set __demir_intrin = 315

/proc/nameof(X)
	set __demir_intrin = 400

/proc/nonspantext(Haystack,Needles,Start=1)
	set __demir_intrin = 316

/proc/num2text(N,SigFig=6,Radix)
	set __demir_intrin = 317

/proc/obounds(Ref=src,Dist=0)
	set __demir_intrin = 318

/proc/ohearers(Depth=world.view,Center=usr)
	set __demir_intrin = 319

/proc/orange(Dist,Center=usr)
	set __demir_intrin = 320

/proc/output(msg,control)
	set __demir_intrin = 321

/proc/oview(Dist,Center=usr)
	set __demir_intrin = 322

/proc/oviewers(Depth=world.view,Center=usr)
	set __demir_intrin = 323

/proc/params2list(Params)
	set __demir_intrin = 324

/proc/pixloc(x,y,z)
	set __demir_intrin = 325

/proc/prob(P)
	set __demir_intrin = 326

/proc/rand(L=0,H)
	set __demir_intrin = 327

/proc/rand_seed(Seed)
	set __demir_intrin = 328

/proc/range(Dist,Center=usr)
	set __demir_intrin = 329

/proc/ref(A)
	set __demir_intrin = 330

/proc/refcount(Object)
	set __demir_intrin = 331

/proc/regex(pattern,flags)
	set __demir_intrin = 332

/proc/replacetext_char(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = 333

/proc/replacetextEx_char(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = 334

/proc/replacetextEx(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = 335

/proc/replacetext(Haystack,Needle,Replacement,Start=1,End=0)
	set __demir_intrin = 336

/proc/rgb2num(color,space)
	set __demir_intrin = 337

/proc/rgb(R,G,B,A=null,space,red,blue,green,alpha,h,hue,s,saturation,c,chroma,v,value,l,y,luminance)
	set __demir_intrin = 338

/proc/roll(ndice=1,sides)
	set __demir_intrin = 339

/proc/round(A,B=null)
	set __demir_intrin = 340

/proc/run(File)
	set __demir_intrin = 341

/proc/sha1(StringOrFile)
	set __demir_intrin = 342

/proc/shell(Command)
	set __demir_intrin = 343

/proc/shutdown(Addr,Natural=0)
	set __demir_intrin = 344

/proc/sign(A)
	set __demir_intrin = 345

/proc/sin(X)
	set __demir_intrin = 346

/proc/sleep(Delay)
	set __demir_intrin = 347

/proc/sorttextEx(T1,T2)
	set __demir_intrin = 348

/proc/sorttext(T1,T2)
	set __demir_intrin = 349

/proc/sound(file,repeat=0,wait,channel,volume)
	set __demir_intrin = 350

/proc/spantext_char(Haystack,Needles,Start=1)
	set __demir_intrin = 351

/proc/spantext(Haystack,Needles,Start=1)
	set __demir_intrin = 352

/proc/splicetext_char(Text,Start=1,End=0,Insert="")
	set __demir_intrin = 353

/proc/splicetext(Text,Start=1,End=0,Insert="")
	set __demir_intrin = 354

/proc/splittext_char(Text,Delimiter,Start=1,End=0,include_delimiters=0)
	set __demir_intrin = 355

/proc/splittext(Text,Delimiter,Start=1,End=0,include_delimiters=0)
	set __demir_intrin = 356

/proc/sqrt(A)
	set __demir_intrin = 357

/proc/startup(File,Port=0,Options)
	set __demir_intrin = 358

/proc/stat(Name,Value)
	set __demir_intrin = 359

/proc/statpanel(Panel,Name,Value)
	set __demir_intrin = 360

/proc/step_away(Ref,Trg,Max=5,Speed=0)
	set __demir_intrin = 361

/proc/step_rand(Ref,Speed=0)
	set __demir_intrin = 362

/proc/step(Ref,Dir,Speed=0)
	set __demir_intrin = 363

/proc/step_to(Ref,Trg,Min=0,Speed=0)
	set __demir_intrin = 364

/proc/step_towards(Ref,Trg,Speed)
	set __demir_intrin = 365

/proc/tan(A)
	set __demir_intrin = 366

/proc/text2ascii_char(T,pos=1)
	set __demir_intrin = 367

/proc/text2ascii(T,pos=1)
	set __demir_intrin = 368

/proc/text2file(Text,File)
	set __demir_intrin = 369

/proc/text2num(T,Radix)
	set __demir_intrin = 370

/proc/text2path(T)
	set __demir_intrin = 371

/proc/text(FormatText,Args)
	set __demir_intrin = 372

/proc/time2text(timestamp,format,timezone)
	set __demir_intrin = 373

/proc/trimtext(Text)
	set __demir_intrin = 374

/proc/trunc(A)
	set __demir_intrin = 375

/proc/turn(Dir,Angle)
	set __demir_intrin = 376

/proc/typesof(Type1,Type2)
	set __demir_intrin = 377

/proc/uppertext(T)
	set __demir_intrin = 378

/proc/url_decode(UrlText)
	set __demir_intrin = 379

/proc/url_encode(PlainText,format=0)
	set __demir_intrin = 380

/proc/values_cut_over(Alist,Max,inclusive=0)
	set __demir_intrin = 381

/proc/values_cut_under(Alist,Max,inclusive=0)
	set __demir_intrin = 382

/proc/values_dot(A,B)
	set __demir_intrin = 383

/proc/values_product(Alist)
	set __demir_intrin = 384

/proc/values_sum(Alist)
	set __demir_intrin = 385

/proc/vector(x,y,z)
	set __demir_intrin = 386

/proc/view(Dist=5,Center=usr)
	set __demir_intrin = 387

/proc/viewers(Depth=world.view,Center=usr)
	set __demir_intrin = 388

/proc/walk_away(Ref,Trg,Max=5,Lag=0,Speed=0)
	set __demir_intrin = 389

/proc/walk_rand(Ref,Lag=0,Speed=0)
	set __demir_intrin = 390

/proc/walk(Ref,Dir,Lag=0,Speed=0)
	set __demir_intrin = 391

/proc/walk_to(Ref,Trg,Min=0,Lag=0,Speed=0)
	set __demir_intrin = 392

/proc/walk_towards(Ref,Trg,Lag=0,Speed=0)
	set __demir_intrin = 393

/proc/winclone(player,window_name,clone_name)
	set __demir_intrin = 394

/proc/winexists(player,control_id)
	set __demir_intrin = 395

/proc/winget(player,control_id,params)
	set __demir_intrin = 396

/proc/winset(player,control_id,params)
	set __demir_intrin = 397

/proc/winshow(player,window,show=1)
	set __demir_intrin = 398

///
/// NATIVE HOOKS
///

/proc/_dm_new_icon(icon,icon_state,dir,frame,moving)
	set __demir_intrin = 600

/proc/_dm_turn_icon(icon,angle,antialias)
	set __demir_intrin = 601

/proc/_dm_flip_icon(icon,dir)
	set __demir_intrin = 602

/proc/_dm_shift_icon(icon,dir,offset,wrap)
	set __demir_intrin = 603

/proc/_dm_icon_intensity(icon,r,g,b)
	set __demir_intrin = 604

/proc/_dm_icon_blend(icon,other,function,x,y)
	set __demir_intrin = 605

/proc/_dm_icon_swap_color(icon,old_rgb,new_rgb)
	set __demir_intrin = 606

/proc/_dm_icon_draw_box(icon,rgb,x1,y1,x2,y2)
	set __demir_intrin = 607

/proc/_dm_icon_insert(icon,new_icon,icon_state,dir,frame,moving,delay)
	set __demir_intrin = 608

/proc/_dm_icon_map_colors(icon,...)
	set __demir_intrin = 609

/proc/_dm_icon_scale(icon,width,height)
	set __demir_intrin = 610

/proc/_dm_icon_crop(icon,x1,y1,x2,y2)
	set __demir_intrin = 611

/proc/_dm_icon_getpixel(icon,x,y,icon_state,dir,frame,moving)
	set __demir_intrin = 612

/proc/_dm_icon_size(icon,axis)
	set __demir_intrin = 613

/proc/_dm_database(db,operation,argument)
	set __demir_intrin = 620

/proc/make_generator(type,rand,a,b)
	set __demir_intrin = 630
