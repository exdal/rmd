use core::{file_path, path::TreePath, types::Identifier, vars};
use std::{collections::HashSet, path::PathBuf};

use crate::{
    FaultKind, GenericValue, Intrinsic,
    builtins::index_arg,
    eval::Evaluator,
    heap::{Object, ObjectId},
    intrinsic::view_grid::{Sight, ViewGrid, ViewTile, dantom_spiral},
    json, lighting,
    matrix::Matrix,
    value::{ListData, ProcRef, Src},
    world::Position,
};

pub(crate) type Result<T> = std::result::Result<T, crate::Fault>;

impl Evaluator<'_> {
    /// https://ref.dm-lang.org/proc/call
    ///
    /// ProcRef: path of proc (/proc/MyProc)
    /// Object: source of proc or verb
    /// ProcName: name of proc or verb ("MyProc")
    /// LibName: name of external library ("test.DLL")
    /// FuncName: name of function in external library ("func")
    /// Returns: The return value of the proc being called.
    ///
    /// https://ref.dm-lang.org/proc/call_ext
    ///
    /// LibName: name of external library ("test.DLL") (note
    /// FuncName: name of function in external library ("func"), which may have prefixes to describe the type of
    /// function
    /// LoadedFunc: reference to a function that was loaded via load_ext()
    /// Returns: The return value of the external library function.
    pub(crate) fn dm_call(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let (src, ty, proc_name) = match arg(0) {
            GenericValue::Object(id) => (
                Src::Object(id),
                self.runtime.heap.object(id).map(|object| object.ty),
                arg(1).text().map(Identifier::from),
            ),
            GenericValue::List(id) | GenericValue::ArgList(id) => {
                let receiver = Src::List(id);
                (
                    receiver,
                    self.receiver_type(receiver),
                    arg(1).text().map(Identifier::from),
                )
            },
            GenericValue::Path(path) => {
                let owner = TreePath::new(path.declaration_owner().to_vec(), true);
                (
                    if owner.segments.is_empty() {
                        Src::None
                    } else {
                        self.current_receiver
                    },
                    self.tree.id_of(&owner),
                    path.name().cloned(),
                )
            },
            GenericValue::Text(proc_name) if args.len() == 1 => {
                (Src::None, Some(objtree::TypeId::ROOT), Some(proc_name.as_ref().into()))
            },
            _ => return Err(self.fault(FaultKind::Blocked("external call".into()))),
        };
        let proc = ty
            .zip(proc_name)
            .and_then(|(ty, proc_name)| self.find_proc(ty, &proc_name))
            .ok_or_else(|| self.fault(FaultKind::MissingProc("dynamic DM proc".into())))?;
        Ok(GenericValue::Proc(ProcRef { src, proc }))
    }

    /// https://ref.dm-lang.org/proc/arglist
    ///
    /// List: a list to be used as the arguments to a procedure
    pub(crate) fn arglist(&self, value: GenericValue) -> Result<GenericValue> {
        match value {
            GenericValue::List(id) | GenericValue::ArgList(id) => Ok(GenericValue::ArgList(id)),
            _ => Err(self.fault(FaultKind::InvalidReference)),
        }
    }

    pub(crate) fn file2list(&mut self, file: &GenericValue, separator: &GenericValue) -> Result<GenericValue> {
        let Some(path) = self.resource_path(file)? else {
            return Ok(GenericValue::Null);
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(GenericValue::Null);
        };
        if text.len() > self.limits.text_bytes {
            return Err(self.fault(FaultKind::Memory));
        }

        let separator = separator.text().unwrap_or("\n");
        let lines = match separator.is_empty() {
            true => vec![text.as_str()],
            false => text.split(separator).collect(),
        };

        self.reserve(lines.len())?;
        let entries = lines
            .iter()
            .map(|line| (GenericValue::Text(line.trim_end_matches('\r').into()), None))
            .collect::<Vec<_>>();

        self.runtime
            .heap
            .alloc_list(ListData::new(entries))
            .map(GenericValue::List)
            .map_err(|kind| self.fault(kind))
    }

    /// https://ref.dm-lang.org/proc/block
    ///
    /// Returns: The list of turfs in the 3D block defined by Start and End (inclusive).
    ///
    /// `block(Start, End)` or `block(x1, y1, z1, x2 = x1, y2 = y1, z2 = z1)`
    pub(crate) fn block(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        self.is_cacheable = false;
        let (start, end) = match args {
            [GenericValue::Object(start), GenericValue::Object(end), ..] => {
                let world = &self.runtime.world;
                match (
                    world.position(&self.runtime.heap, *start),
                    world.position(&self.runtime.heap, *end),
                ) {
                    (Some(start), Some(end)) => ([start.x, start.y, start.z], [end.x, end.y, end.z]),
                    _ => return self.list(Vec::new()),
                }
            },
            _ => {
                let mut start = [0; 3];
                for (axis, value) in start.iter_mut().enumerate() {
                    *value = self.number(args.get(axis).unwrap_or(&GenericValue::Null))? as i32;
                }
                let mut end = start;
                for (axis, value) in end.iter_mut().enumerate() {
                    match args.get(axis + 3) {
                        None | Some(GenericValue::Null) => {},
                        Some(coordinate) => *value = self.number(coordinate)? as i32,
                    }
                }
                (start, end)
            },
        };

        let size = self.runtime.world.size;
        let low = std::array::from_fn::<i32, 3, _>(|axis| start[axis].min(end[axis]).max(1));
        let high = std::array::from_fn::<i32, 3, _>(|axis| start[axis].max(end[axis]).min(size[axis]));
        let count = (0..3)
            .map(|axis| usize::try_from(i64::from(high[axis]) - i64::from(low[axis]) + 1).unwrap_or(0))
            .product::<usize>();
        self.charge(count)?;

        let mut entries = Vec::with_capacity(count);
        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    if let Some(turf) = self.runtime.world.turf_at(Position::new(x, y, z)) {
                        entries.push((GenericValue::Object(turf), None));
                    }
                }
            }
        }

        self.list(entries)
    }

    /// https://ref.dm-lang.org/proc/range
    ///
    /// Returns: A list of objects within Dist tiles of Center.
    ///
    /// https://ref.dm-lang.org/proc/orange
    ///
    /// Returns: A list of objects within Dist tiles of Center, excluding Center.
    pub(crate) fn range(&mut self, args: [GenericValue; 2], include_center: bool) -> Result<GenericValue> {
        self.range_list(args, include_center, Sight::Through)
    }

    /// https://ref.dm-lang.org/proc/view
    ///
    /// Returns: A list of visible objects within Dist tiles of Center.
    ///
    /// https://ref.dm-lang.org/proc/oview
    ///
    /// Returns: A list of visible objects within Dist tiles of Center, excluding Center.
    pub(crate) fn view(&mut self, args: [GenericValue; 2], include_center: bool) -> Result<GenericValue> {
        self.range_list(args, include_center, Sight::Line)
    }

    pub(crate) fn dm_icon_size(&self, icon_file: GenericValue, axis: f32) -> Result<GenericValue> {
        let icon = self.icon_file(icon_file);
        let size = match &icon {
            GenericValue::Resource(path) | GenericValue::Text(path) => self.runtime.icons.size(path),
            _ => None,
        }
        .ok_or_else(|| self.fault(FaultKind::Unsupported(format!("size of icon {}", icon.display()))))?;

        Ok((size[usize::from(axis == 2.0)] as f32).into())
    }

    /// https://ref.dm-lang.org/proc/icon_states
    ///
    /// Returns: A list of text strings.
    pub(crate) fn icon_states(&mut self, icon_file: GenericValue) -> Result<GenericValue> {
        let icon = self.icon_file(icon_file);
        let states = match &icon {
            GenericValue::Resource(path) | GenericValue::Text(path) => {
                self.runtime.icons.get(path).map(<[_]>::to_vec).unwrap_or_default()
            },
            _ => Vec::new(),
        };
        self.charge(states.len())?;

        self.list(
            states
                .into_iter()
                .map(|state| (GenericValue::Text(state), None))
                .collect(),
        )
    }

    /// https://ref.dm-lang.org/proc/issaved
    ///
    /// This returns 1 if the given variable should be automatically saved when writing an object to a savefile and 0
    /// otherwise. Variables which are not global, const, or tmp will return 1.
    pub(crate) fn is_saved(&self, owner: &GenericValue, name: &GenericValue) -> bool {
        let Some(name) = name.text() else {
            return true;
        };

        if matches!(name, vars::TYPE | vars::PARENT_TYPE | vars::VARS) {
            return false;
        }

        let ty = match owner {
            GenericValue::Object(id) => self.runtime.heap.object(*id).map(|object| object.ty),
            GenericValue::Path(path) => self.tree.id_of(path),
            _ => None,
        };
        let Some((_, variable)) = ty.and_then(|ty| self.tree.var_declaration(ty, &Identifier::from(name))) else {
            return true;
        };

        let modifiers = variable.modifiers;
        !(modifiers.is_global || modifiers.is_static || modifiers.is_const || modifiers.is_tmp)
    }

    /// https://ref.dm-lang.org/proc/nameof
    ///
    /// Var: A variable, e.g. src.density or foo
    /// ProcRef: A proc reference, e.g. /mob
    /// Path: A type path, e.g. /obj/item/barrel.
    /// This returns the name of a var or proc, or the last part of a type path. This proc only exists at compile-time.
    pub(crate) fn nameof(&mut self, value: GenericValue) -> Result<GenericValue> {
        match value {
            GenericValue::Path(path) => match path.name() {
                Some(name) => self.text(name.to_string()),
                None => Ok(GenericValue::Null),
            },
            _ => Err(self.fault(FaultKind::Blocked("nameof of a non-path".into()))),
        }
    }

    /// https://ref.dm-lang.org/proc/file2text
    ///
    /// File: file to read
    pub(crate) fn file2text(&mut self, file: &GenericValue) -> Result<GenericValue> {
        match self.resource_path(file)? {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(text) if text.len() > self.limits.text_bytes => Err(self.fault(FaultKind::Memory)),
                Ok(text) => self.text(text),
                Err(_) => Ok(GenericValue::Null),
            },
            None => Ok(GenericValue::Null),
        }
    }

    /// https://ref.dm-lang.org/proc/fexists
    ///
    /// File: name of file to test
    pub(crate) fn fexists(&mut self, file: &GenericValue) -> Result<GenericValue> {
        Ok(self.resource_path(file)?.is_some_and(|path| path.is_file()).into())
    }

    /// https://ref.dm-lang.org/proc/file
    pub(crate) fn file(&mut self, file: &GenericValue, name: &str) -> Result<GenericValue> {
        match self.resource_path(file)? {
            Some(_) => Ok(GenericValue::Resource(name.into())),
            None => Ok(GenericValue::Null),
        }
    }

    /// https://ref.dm-lang.org/proc/flist
    ///
    /// Path: The path in the filesystem to get a listing of.
    pub(crate) fn flist(&mut self, file: &GenericValue) -> Result<GenericValue> {
        let Some(path) = self.resource_path(file)? else {
            return Ok(GenericValue::Null);
        };
        let Ok(entries) = std::fs::read_dir(&path) else {
            return self.list(Vec::new());
        };

        let mut names = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().into_string().ok()?;

                Some(match entry.path().is_dir() {
                    true => format!("{name}/"),
                    false => name,
                })
            })
            .collect::<Vec<_>>();
        names.sort();
        self.reserve(names.len())?;

        let entries = names.into_iter().map(|name| (GenericValue::from(name), None)).collect();
        self.list(entries)
    }

    /// https://ref.dm-lang.org/proc/isfile
    ///
    /// File: the value to test
    pub(crate) fn isfile(&self, value: &GenericValue) -> bool {
        matches!(value, GenericValue::Resource(_))
    }

    /// https://ref.dm-lang.org/proc/text
    ///
    /// Returns: The text with macros arguments substituted.
    pub(crate) fn dm_text(&mut self, value: GenericValue) -> Result<GenericValue> {
        match value {
            text @ GenericValue::Text(_) => Ok(text),
            value => self.text(value.display()),
        }
    }

    /// https://ref.dm-lang.org/proc/isnull
    ///
    /// Returns: 1 if Val is null; 0 otherwise
    pub(crate) fn isnull(&self, value: &GenericValue) -> bool {
        matches!(value, GenericValue::Null)
    }

    /// https://ref.dm-lang.org/proc/isnum
    ///
    /// Returns: 1 if Val is a number; 0 otherwise
    pub(crate) fn isnum(&self, value: &GenericValue) -> bool {
        matches!(value, GenericValue::Num(_))
    }

    /// https://ref.dm-lang.org/proc/istext
    ///
    /// Returns: 1 if Val is text; 0 otherwise
    pub(crate) fn istext(&self, value: &GenericValue) -> bool {
        matches!(value, GenericValue::Text(_))
    }

    /// https://ref.dm-lang.org/proc/islist
    ///
    /// Object: The value to test
    pub(crate) fn islist(&self, value: &GenericValue) -> bool {
        matches!(value, GenericValue::List(_) | GenericValue::ArgList(_))
    }

    /// https://ref.dm-lang.org/proc/isicon
    ///
    /// Icon: the value to test
    pub(crate) fn isicon(&self, value: GenericValue) -> bool {
        match value {
            GenericValue::Resource(path) => path.rsplit_once('.').is_some_and(|(_, extension)| {
                ["dmi", "bmp", "png", "jpg", "gif"]
                    .iter()
                    .any(|format| extension.eq_ignore_ascii_case(format))
            }),
            GenericValue::Object(id) => self
                .runtime
                .heap
                .object(id)
                .zip(self.tree.id_of(&TreePath::parse("/icon")))
                .is_some_and(|(object, icon)| self.tree.is_subtype_of(object.ty, icon)),
            _ => false,
        }
    }

    /// https://ref.dm-lang.org/proc/ispath
    ///
    /// Val: A type path.
    /// Type: A type path or instance.
    pub(crate) fn ispath(&self, value: GenericValue, base: GenericValue) -> bool {
        let ty = match value {
            GenericValue::Path(path) => self.tree.id_of(&path),
            _ => None,
        };
        let base = match base {
            GenericValue::Path(path) => self.tree.id_of(&path),
            _ => None,
        };
        ty.is_some_and(|ty| base.is_none_or(|base| self.tree.is_subtype_of(ty, base)))
    }

    /// https://ref.dm-lang.org/proc/istype
    ///
    /// Returns: 1 if Val is derived from Type; 0 otherwise.
    pub(crate) fn istype(&self, value: GenericValue, base: GenericValue) -> bool {
        let base_path = match base {
            GenericValue::Path(path) => Some(*path),
            _ => None,
        };

        match &value {
            GenericValue::Path(path) => self.tree.id_of(path).is_some_and(|ty| {
                base_path
                    .as_ref()
                    .and_then(|base| self.tree.id_of(base))
                    .is_none_or(|base| self.tree.is_subtype_of(ty, base))
            }),
            GenericValue::Object(_) | GenericValue::List(_) | GenericValue::ArgList(_) => {
                base_path.is_none() || self.matches_type(&value, &base_path)
            },
            _ => false,
        }
    }

    /// https://ref.dm-lang.org/proc/isturf
    ///
    /// Any number of locations to test.
    ///
    /// https://ref.dm-lang.org/proc/isarea
    ///
    /// Any number of locations to test.
    ///
    /// https://ref.dm-lang.org/proc/ismovable
    ///
    /// Any number of locations to test.
    ///
    /// https://ref.dm-lang.org/proc/isobj
    ///
    /// Any number of locations to test.
    ///
    /// https://ref.dm-lang.org/proc/ismob
    ///
    /// Any number of locations to test.
    ///
    /// https://ref.dm-lang.org/proc/isloc
    ///
    /// Any number of locations to test.
    pub(crate) fn is_atom_kind(&self, intrinsic: Intrinsic, value: &GenericValue) -> bool {
        let roots = self.tree.roots();
        let root = match intrinsic {
            Intrinsic::Isturf => roots.turf,
            Intrinsic::Isarea => roots.area,
            Intrinsic::Ismovable => roots.movable,
            Intrinsic::Isobj => roots.obj,
            Intrinsic::Ismob => roots.mob,
            _ => roots.atom,
        };

        self.is_root_type(value, root)
    }

    /// https://ref.dm-lang.org/proc/length
    ///
    /// Returns: The length of the data associated with E.
    pub(crate) fn length(&self, value: GenericValue) -> f32 {
        (match value {
            GenericValue::List(id) | GenericValue::ArgList(id) => {
                self.runtime.heap.list(id).map_or(0, |l| l.entries.len())
            },
            GenericValue::Text(s) | GenericValue::Resource(s) => s.chars().count(),
            GenericValue::Null => 0,
            GenericValue::Object(id) => self.runtime.heap.object(id).map_or(0, |o| o.contents.len()),
            _ => 0,
        }) as f32
    }

    /// https://ref.dm-lang.org/proc/typesof
    ///
    /// Returns: A list of all types that are derived from the specified "base" types, including the base types
    /// themselves.
    pub(crate) fn typesof(&mut self, args: Vec<GenericValue>) -> Result<GenericValue> {
        self.types_of(args, true)
    }

    /// https://ref.dm-lang.org/proc/text2path
    ///
    /// T: A text string.
    pub(crate) fn text2path(&self, path: &str) -> GenericValue {
        let path = TreePath::parse(path);
        self.tree
            .id_of(&path)
            .map(|_| GenericValue::Path(Box::new(path)))
            .unwrap_or_default()
    }

    /// https://ref.dm-lang.org/proc/min
    ///
    /// Returns: the minimum of the arguments.
    ///
    /// https://ref.dm-lang.org/proc/max
    ///
    /// Returns: the maximum of the arguments.
    pub(crate) fn min_max(&mut self, intrinsic: Intrinsic, args: Vec<GenericValue>) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let values = if args.len() == 1 && matches!(arg(0), GenericValue::List(_) | GenericValue::ArgList(_)) {
            self.iter_values(arg(0))?.into_iter().map(|(v, _)| v).collect()
        } else {
            args
        };
        let mut result: Option<f32> = None;
        for value in values {
            let n = self.number(&value)?;
            result = Some(result.map_or(n, |r| {
                if intrinsic == Intrinsic::Min {
                    r.min(n)
                } else {
                    r.max(n)
                }
            }));
        }
        Ok(result.unwrap_or(0.0).into())
    }

    /// https://ref.dm-lang.org/proc/abs
    ///
    /// Returns: The absolute value of A.
    pub(crate) fn abs(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.abs().into())
    }

    /// https://ref.dm-lang.org/proc/ceil
    ///
    /// Returns: the ceiling of A
    pub(crate) fn ceil(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.ceil().into())
    }

    /// https://ref.dm-lang.org/proc/floor
    ///
    /// Returns: the floor of A
    pub(crate) fn floor(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.floor().into())
    }

    /// https://ref.dm-lang.org/proc/sqrt
    ///
    /// Returns: The square root of A
    pub(crate) fn sqrt(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.sqrt().into())
    }

    /// https://ref.dm-lang.org/proc/round
    ///
    /// Returns: rounded A
    pub(crate) fn round(&self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let n = self.number(&arg(0))?;
        if args.len() < 2 {
            Ok(n.floor().into())
        } else {
            let step = self.number(&arg(1))?;
            Ok(if step == 0.0 {
                n
            } else {
                (n / step + 0.5).floor() * step
            }
            .into())
        }
    }

    /// https://ref.dm-lang.org/proc/clamp
    ///
    /// Value: A number, text, pixloc, or vector (or null, treated as 0).
    /// List: A list of values.
    /// Low: The lowest value that can be returned.
    /// High: The highest value that can be returned.
    /// Returns: "Clamps" a value, usually a number, to a given allowable range from Low to High. If the value is
    /// already in that range, it is unchanged. Otherwise, the closer of Low or High is returned.
    pub(crate) fn clamp(&self, value: &GenericValue, lo: &GenericValue, hi: &GenericValue) -> Result<GenericValue> {
        let n = self.number(value)?;
        let lo = self.number(lo)?;
        let hi = self.number(hi)?;
        if lo.is_nan() || hi.is_nan() || lo > hi {
            return Err(self.fault(FaultKind::InvalidOperation("invalid clamp bounds".into())));
        }

        Ok(n.clamp(lo, hi).into())
    }

    /// https://ref.dm-lang.org/proc/rand
    ///
    /// Returns: A random integer between L and H inclusive; or a random number from 0 up to 1 if L and H are omitted.
    pub(crate) fn rand(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let (lo, hi) = match args.len() {
            0 => (0.0, 1.0),
            1 => (0.0, self.number(&arg(0))?),
            _ => (self.number(&arg(0))?, self.number(&arg(1))?),
        };
        let empty = args.is_empty();
        let r = self.random();

        Ok(if empty {
            r
        } else {
            lo.min(hi) + (r * ((hi - lo).abs() + 1.0)).floor()
        }
        .into())
    }

    /// https://ref.dm-lang.org/proc/prob
    ///
    /// Returns: 1 with probability P percent; otherwise 0
    pub(crate) fn prob(&mut self, value: &GenericValue) -> Result<GenericValue> {
        let n = self.number(value)?;

        Ok((self.random() * 100.0 < n).into())
    }

    /// https://ref.dm-lang.org/proc/get_step
    ///
    /// Returns: The location of the new position.
    pub(crate) fn get_step(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let dir = self.number(&arg(1))? as i32;
        let query = arg(0)
            .object()
            .and_then(|id| self.runtime.world.position(&self.runtime.heap, id))
            .map(|p| p.step(dir));
        let origin = self
            .origin
            .and_then(|id| self.runtime.world.position(&self.runtime.heap, id));
        if let (Some(a), Some(b)) = (origin, query)
            && (a.z != b.z || a.x.abs_diff(b.x) > 1 || a.y.abs_diff(b.y) > 1)
        {
            self.is_cacheable = false;
        }

        Ok(arg(0)
            .object()
            .and_then(|id| self.runtime.world.get_step(&self.runtime.heap, id, dir))
            .map(GenericValue::Object)
            .unwrap_or_default())
    }

    /// https://ref.dm-lang.org/proc/get_dir
    ///
    /// Returns: The direction from Loc1 to Loc2. Possible results are NORTH, SOUTH, EAST, WEST, NORTHEAST, NORTHWEST,
    /// SOUTHEAST, and SOUTHWEST.
    ///
    /// https://ref.dm-lang.org/proc/get_dist
    ///
    /// Returns: The distance between Loc1 and Loc2, in tiles. This is the number of full-tile movements (disregarding
    /// any obstacles and allowing diagonal moves including across Z levels) required to go from one to the other. You
    /// can think of it as the max of their x, y, and z distances.
    pub(crate) fn get_dir_dist(
        &mut self, intrinsic: Intrinsic, from: GenericValue, to: GenericValue,
    ) -> Result<GenericValue> {
        self.position_sensitive = true;
        let pos = |value: GenericValue, evaluator: &Self| {
            value
                .object()
                .and_then(|id| evaluator.runtime.world.position(&evaluator.runtime.heap, id))
        };
        let (Some(a), Some(b)) = (pos(from, self), pos(to, self)) else {
            return Ok(0.0.into());
        };

        if intrinsic == Intrinsic::GetDir {
            Ok(((i32::from(b.x > a.x) * 4)
                | (i32::from(b.x < a.x) * 8)
                | i32::from(b.y > a.y)
                | (i32::from(b.y < a.y) * 2))
                .into())
        } else {
            Ok(if a.z != b.z {
                -1.0
            } else {
                a.x.abs_diff(b.x).max(a.y.abs_diff(b.y)) as f32
            }
            .into())
        }
    }

    /// https://ref.dm-lang.org/proc/locate
    ///
    /// Returns: An object of the specified type or the turf at the given coordinates. If a text string is given in
    /// place of an object type, the object with the same tag is found. If a container is given, only objects directly
    /// within that object are searched.
    pub(crate) fn locate(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        if args.len() >= 3 || args.len() < 2 {
            self.is_cacheable = false;
        }

        if args.len() >= 3 {
            let pos = Position::new(
                self.number(&arg(0))? as i32,
                self.number(&arg(1))? as i32,
                self.number(&arg(2))? as i32,
            );

            return Ok(self
                .runtime
                .world
                .turf_at(pos)
                .map(GenericValue::Object)
                .unwrap_or_default());
        }

        let values = self.iter_values(args.get(1).cloned().unwrap_or(GenericValue::World))?;
        let query = arg(0);
        let base = if let GenericValue::Path(path) = &query {
            self.tree.id_of(path)
        } else {
            None
        };

        Ok(values
            .into_iter()
            .find(|(v, _)| {
                base.map_or(*v == query, |base| {
                    v.object()
                        .and_then(|id| self.runtime.heap.object(id))
                        .is_some_and(|o| self.tree.is_subtype_of(o.ty, base))
                })
            })
            .map(|(v, _)| v)
            .unwrap_or_default())
    }

    /// https://ref.dm-lang.org/proc/turn
    ///
    /// Dir: One of NORTH, SOUTH, EAST, WEST, NORTHEAST, NORTHWEST, SOUTHEAST, SOUTHWEST.
    /// Angle: An angle in degrees (counterclockwise rotation).
    /// Returns: The rotated direction.
    pub(crate) fn turn(&mut self, value: &GenericValue, angle: &GenericValue) -> Result<GenericValue> {
        if let Some(matrix) = self.matrix(value) {
            let angle = self.number(angle)?;

            return self.store_matrix(None, matrix.then(Matrix::turn(angle)));
        }

        let dirs = [1, 9, 8, 10, 2, 6, 4, 5];
        let dir = self.number(value)? as i32;
        let angle = self.number(angle)?;
        let Some(index) = dirs.iter().position(|d| *d == dir) else {
            return Ok(dir.into());
        };
        let index = (index as i32 + (angle.rem_euclid(360.0) / 45.0).round() as i32).rem_euclid(8) as usize;

        Ok(dirs.get(index).copied().unwrap_or(0).into())
    }

    /// https://ref.dm-lang.org/proc/rgb
    ///
    /// R,G,B: Numbers from 0-255 corresponding to the red, green, and blue components of a color.
    /// A: Optional alpha component; 0 is transparent, 255 is opaque.
    /// x,y,z: Color components for a different color space
    /// space: Color space; defaults to COLORSPACE_RGB
    /// Returns: A color, represented by a text string in #RRGGBB or #RRGGBBAA format
    pub(crate) fn rgb(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let mut color = String::from("#");
        for index in 0..args.len() {
            let channel = self.number(&arg(index))?.round().clamp(0.0, 255.0) as u8;
            color.push_str(&format!("{channel:02x}"));
        }

        self.text(color)
    }

    /// https://ref.dm-lang.org/proc/rgb2num
    ///
    /// color: A color value (see HTML colors)
    /// space: Color space; default is COLORSPACE_RGB
    /// Returns: A list with the components of this color
    pub(crate) fn rgb2num(&mut self, name: &str, color: GenericValue) -> Result<GenericValue> {
        let Some((bytes, alpha)) = color.text().and_then(lighting::color_bytes) else {
            return Err(self.fault(FaultKind::Blocked(name.into())));
        };
        let channels = if alpha { &bytes[..] } else { &bytes[..3] };

        self.list(
            channels
                .iter()
                .map(|channel| (GenericValue::Num(f32::from(*channel)), None))
                .collect(),
        )
    }

    /// https://ref.dm-lang.org/proc/num2text
    ///
    /// Returns: A text string.
    pub(crate) fn num2text(&mut self, value: &GenericValue) -> Result<GenericValue> {
        let n = self.number(value)?;

        self.text(n.to_string())
    }

    /// https://ref.dm-lang.org/proc/text2num
    ///
    /// T: A text string.
    /// radix: The radix/base of the number, e.g. 16 for hexadecimal
    /// Returns: A number.
    pub(crate) fn text2num(&self, text: &str) -> GenericValue {
        text.trim().parse::<f32>().map(GenericValue::Num).unwrap_or_default()
    }

    /// https://ref.dm-lang.org/proc/uppertext
    ///
    /// Returns: A capitalized text string.
    pub(crate) fn uppertext(&mut self, text: &str) -> Result<GenericValue> {
        let t = text.to_uppercase();

        self.text(t)
    }

    /// https://ref.dm-lang.org/proc/lowertext
    ///
    /// Returns: A lowercase text string.
    pub(crate) fn lowertext(&mut self, text: &str) -> Result<GenericValue> {
        let t = text.to_lowercase();

        self.text(t)
    }

    /// https://ref.dm-lang.org/proc/sorttext
    ///
    /// Any number of text strings to sort.
    ///
    /// https://ref.dm-lang.org/proc/sorttextEx
    ///
    /// Any number of text strings to sort.
    pub(crate) fn sorttext(&mut self, intrinsic: Intrinsic, first: &str, second: &str) -> Result<GenericValue> {
        self.charge(first.len().saturating_add(second.len()))?;
        let order = if intrinsic == Intrinsic::SorttextEx {
            second.cmp(first)
        } else {
            second.to_lowercase().cmp(&first.to_lowercase())
        };

        Ok((order as i32).into())
    }

    /// https://ref.dm-lang.org/proc/copytext
    ///
    /// Returns: A text string.
    pub(crate) fn copytext(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let text = |n: usize| args.get(n).and_then(|value| value.text()).unwrap_or_default();

        let s = text(0);
        let chars = s.chars().collect::<Vec<_>>();
        self.charge(chars.len())?;
        let start = index_arg(args, 1, 1, chars.len() + 1);
        let end = index_arg(args, 2, 0, chars.len() + 1);

        self.text(chars.get(start.min(end)..end).unwrap_or_default().iter().collect())
    }

    /// https://ref.dm-lang.org/proc/findtext
    ///
    /// Returns: The first position of Needle in Haystack; 0 if not found.
    ///
    /// https://ref.dm-lang.org/proc/findtextEx
    ///
    /// Returns: The position of Needle in Haystack; 0 if not found.
    ///
    /// https://ref.dm-lang.org/proc/findlasttext
    ///
    /// Returns: The last position of Needle in Haystack; 0 if not found.
    ///
    /// https://ref.dm-lang.org/proc/findlasttextEx
    ///
    /// Returns: The last position of Needle in Haystack; 0 if not found.
    pub(crate) fn findtext(&mut self, intrinsic: Intrinsic, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|value| value.text()).unwrap_or_default();

        if matches!(arg(1), GenericValue::Object(_)) {
            return Err(self.fault(FaultKind::Blocked("regex matching".into())));
        }

        let case = matches!(intrinsic, Intrinsic::FindtextEx | Intrinsic::FindlasttextEx);
        let last = matches!(intrinsic, Intrinsic::Findlasttext | Intrinsic::FindlasttextEx);
        let hay = if case {
            text(0).to_string()
        } else {
            text(0).to_lowercase()
        };
        let needle = if case {
            text(1).to_string()
        } else {
            text(1).to_lowercase()
        };
        self.charge(hay.len().saturating_add(needle.len()))?;
        let chars = hay.chars().collect::<Vec<_>>();
        let start = index_arg(args, 2, 1, chars.len() + 1);
        let end = index_arg(args, 3, 0, chars.len() + 1);
        let segment: String = chars.get(start.min(end)..end).unwrap_or_default().iter().collect();
        let found = if last {
            segment.rfind(&needle)
        } else {
            segment.find(&needle)
        };

        Ok(found
            .map(|offset| start + segment.get(..offset).unwrap_or_default().chars().count() + 1)
            .unwrap_or(0)
            .into())
    }

    /// https://ref.dm-lang.org/proc/replacetext
    ///
    /// Returns: The Haystack text with all cases of Needle replaced by Replacement
    ///
    /// https://ref.dm-lang.org/proc/replacetextEx
    ///
    /// Returns: The Haystack text with all cases of Needle replaced by Replacement
    pub(crate) fn replacetext(&mut self, intrinsic: Intrinsic, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|value| value.text()).unwrap_or_default();

        if matches!(arg(1), GenericValue::Object(_)) {
            return Err(self.fault(FaultKind::Blocked("regex matching".into())));
        }

        let source = text(0);
        let needle = text(1);
        let replacement = text(2);
        if needle.is_empty() {
            return self.text(source.to_string());
        }

        let mut output = String::new();
        let mut offset = 0;
        while let Some(rest) = source.get(offset..) {
            self.charge(1)?;
            if rest.is_empty() {
                break;
            }

            let candidate = rest.get(..needle.len());
            let found = candidate.is_some_and(|s| {
                if intrinsic == Intrinsic::ReplacetextEx {
                    s == needle
                } else {
                    s.eq_ignore_ascii_case(needle)
                }
            });
            let piece = if found {
                offset += needle.len();
                replacement
            } else {
                let length = rest.chars().next().map(char::len_utf8).unwrap_or(1);
                offset += length;
                rest.get(..length).unwrap_or_default()
            };
            if output.len().saturating_add(piece.len()) > self.limits.text_bytes {
                return Err(self.fault(FaultKind::Memory));
            }

            output.push_str(piece);
        }

        self.text(output)
    }

    /// https://ref.dm-lang.org/proc/splittext
    ///
    /// Returns: A list of text strings split by the delimiter given.
    pub(crate) fn splittext(&mut self, source: &str, delimiter: &str) -> Result<GenericValue> {
        self.charge(source.len())?;
        let entries = if delimiter.is_empty() {
            source
                .chars()
                .map(|c| (GenericValue::from(c.to_string()), None))
                .collect()
        } else {
            source.split(delimiter).map(|s| (GenericValue::from(s), None)).collect()
        };

        self.list(entries)
    }

    /// https://ref.dm-lang.org/proc/params2list
    ///
    /// Params: Text string of parameter values.
    pub(crate) fn params2list(&mut self, source: &str) -> Result<GenericValue> {
        self.charge(source.len())?;
        let mut entries: Vec<(GenericValue, Option<GenericValue>)> = Vec::new();
        for pair in source.split(['&', ';']).filter(|pair| !pair.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some((key, value)) => (decode_param(key), Some(GenericValue::from(decode_param(value)))),
                None => (decode_param(pair), None),
            };
            let key = GenericValue::from(key);
            match entries.iter_mut().find(|(existing, _)| *existing == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
        }

        self.list(entries)
    }

    /// https://ref.dm-lang.org/proc/json_decode
    ///
    /// Returns: A value interpreted from a JSON-formatted text string.
    pub(crate) fn json_decode(&mut self, text: &str) -> Result<GenericValue> {
        let value = json::decode(
            text,
            self.limits.allocations.min(self.instruction_budget_remaining as usize),
        )
        .map_err(|e| self.fault(e))?;
        self.reserve(text.len())?;

        self.constant(&value)
    }

    /// https://ref.dm-lang.org/proc/ref
    ///
    /// Object: The object to get a reference for
    pub(crate) fn ref_text(&mut self, value: &GenericValue) -> Result<GenericValue> {
        self.is_cacheable = false;

        self.text(value.display())
    }

    /// https://ref.dm-lang.org/proc/hascall
    ///
    /// Object: source of proc or verb
    /// ProcName: name of proc or verb ("MyProc")
    /// Returns: 1 if object has such a proc or verb; 0 otherwise
    pub(crate) fn hascall(&self, value: GenericValue, name: &str) -> bool {
        let name = Identifier::from(name);

        value
            .object()
            .and_then(|id| self.runtime.heap.object(id))
            .and_then(|o| self.find_proc(o.ty, &name))
            .is_some()
    }

    /// https://ref.dm-lang.org/proc/CRASH
    pub(crate) fn crash(&self, message: GenericValue) -> Result<GenericValue> {
        Err(self.fault(FaultKind::InvalidOperation(message.display())))
    }

    /// https://ref.dm-lang.org/proc/icon
    ///
    /// icon: an icon file or /icon object
    /// icon_state: an optional text string, specifying a single icon state to load
    /// dir: an optional direction to extract
    /// frame: an optional animation frame to extract
    /// moving: Non-zero to extract only movement states, 0 for non-movement states, or null (default) for both
    /// This is equivalent to new/icon(). It creates an /icon object, which is initialized to contain the same graphical
    /// data as the given file. If an icon state or direction are specified, only those parts of the original icon will
    /// be included in the new icon object.
    pub(crate) fn icon(
        &mut self, params: &[Identifier], args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        self.construct("/icon", params, None, args)
    }

    /// https://ref.dm-lang.org/proc/sound
    ///
    /// file: A sound file to play
    /// repeat: 1 to play sound repeatedly
    /// wait: 0 to interrupt current sound on channel; 1 to wait in queue
    /// channel: 0 for any available channel, 1-1024 for specific channel (non-MIDI only)
    /// volume: 100 for full volume (default), 0 for none, or any value in between
    /// This is used to play a sound file.
    pub(crate) fn sound(
        &mut self, params: &[Identifier], args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        self.construct("/sound", params, None, args)
    }

    /// https://ref.dm-lang.org/proc/matrix
    ///
    /// Returns: A new matrix.
    pub(crate) fn dm_matrix(&mut self, args: Vec<GenericValue>) -> Result<GenericValue> {
        self.matrix_call(args)
    }

    /// https://ref.dm-lang.org/proc/regex
    ///
    /// Returns: A new /regex datum.
    pub(crate) fn regex(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let (target, pattern, flags) = match arg(0) {
            GenericValue::Object(id) => (Some(id), arg(1), arg(2)),
            _ => (None, arg(0), arg(1)),
        };
        let id = match target {
            Some(id) => id,
            None => {
                let ty = self
                    .tree
                    .id_of(&TreePath::parse("/regex"))
                    .ok_or_else(|| self.fault(FaultKind::MissingVariable("/regex".into())))?;
                self.reserve(1)?;

                self.runtime
                    .heap
                    .alloc_object(Object::new(ty))
                    .map_err(|kind| self.fault(kind))?
            },
        };

        self.write_field(GenericValue::Object(id), vars::NAME.into(), pattern)?;
        self.write_field(GenericValue::Object(id), "flags".into(), flags)?;

        Ok(GenericValue::Object(id))
    }

    /// https://ref.dm-lang.org/proc/trimtext
    ///
    /// Returns: Text with whitespace trimmed from both ends
    pub(crate) fn trimtext(&mut self, text: &str) -> Result<GenericValue> {
        let trimmed = text.trim().to_string();

        self.text(trimmed)
    }

    /// https://ref.dm-lang.org/proc/ckey
    ///
    /// Key: The player key to convert to canonical form.
    ///
    /// https://ref.dm-lang.org/proc/ckeyEx
    ///
    /// Text: The text string to convert to case-sensitive canonical key form.
    pub(crate) fn ckey(&mut self, intrinsic: Intrinsic, text: &str) -> Result<GenericValue> {
        let fold = intrinsic == Intrinsic::Ckey;
        let key = text
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| if fold { c.to_ascii_lowercase() } else { c })
            .collect::<String>();

        self.text(key)
    }

    /// https://ref.dm-lang.org/proc/cmptext
    ///
    /// Returns: 1 if all arguments are equal; 0 otherwise.
    ///
    /// https://ref.dm-lang.org/proc/cmptextEx
    ///
    /// Returns: 1 if all arguments are equal; 0 otherwise.
    pub(crate) fn cmptext(&mut self, intrinsic: Intrinsic, args: &[GenericValue]) -> Result<GenericValue> {
        let text = |n: usize| args.get(n).and_then(|value| value.text()).unwrap_or_default();

        let exact = intrinsic == Intrinsic::CmptextEx;
        let first = text(0);
        self.charge(args.len())?;

        Ok((1..args.len())
            .all(|index| match exact {
                true => text(index) == first,
                false => text(index).eq_ignore_ascii_case(first),
            })
            .into())
    }

    /// https://ref.dm-lang.org/proc/addtext
    ///
    /// Returns: A text string with the arguments concatenated.
    pub(crate) fn addtext(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let mut output = String::new();
        for value in args {
            let piece = value.display();
            if output.len().saturating_add(piece.len()) > self.limits.text_bytes {
                return Err(self.fault(FaultKind::Memory));
            }

            output.push_str(&piece);
        }

        self.text(output)
    }

    /// https://ref.dm-lang.org/proc/jointext
    ///
    /// Returns: A text string made up of the items in List, joined together by Glue.
    pub(crate) fn jointext(&mut self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|value| value.text()).unwrap_or_default();

        let entries = self.iter_values(arg(0))?;
        let glue = text(1).to_string();
        let start = index_arg(args, 2, 1, entries.len() + 1);
        let end = index_arg(args, 3, 0, entries.len() + 1);

        let mut output = String::new();
        for (index, (value, _)) in entries.get(start.min(end)..end).unwrap_or_default().iter().enumerate() {
            let piece = value.display();
            let extra = piece.len() + if index == 0 { 0 } else { glue.len() };
            if output.len().saturating_add(extra) > self.limits.text_bytes {
                return Err(self.fault(FaultKind::Memory));
            }

            if index > 0 {
                output.push_str(&glue);
            }
            output.push_str(&piece);
        }

        self.text(output)
    }

    /// https://ref.dm-lang.org/proc/text2ascii
    ///
    /// T: A text string.
    /// pos: The byte position in T to use, starting at 1.
    /// Returns: A number representing the character's ASCII or Unicode code.
    pub(crate) fn text2ascii(&self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|value| value.text()).unwrap_or_default();

        let position = match args.len() {
            0 | 1 => 1.0,
            _ => self.number(&arg(1))?,
        };
        let chars = text(0).chars().collect::<Vec<_>>();
        let index = index_arg(&[GenericValue::Num(position)], 0, 1, chars.len() + 1);

        Ok(chars.get(index).map(|c| *c as u32 as f32).unwrap_or(0.0).into())
    }

    /// https://ref.dm-lang.org/proc/ascii2text
    ///
    /// Returns: A text string.
    pub(crate) fn ascii2text(&mut self, value: &GenericValue) -> Result<GenericValue> {
        let code = self.number(value)?;
        let Some(character) = u32::try_from(code as i64).ok().and_then(char::from_u32) else {
            return Ok(GenericValue::Null);
        };

        self.text(character.to_string())
    }

    /// https://ref.dm-lang.org/proc/sin
    ///
    /// Returns: The sine of X, where X is in degrees.
    pub(crate) fn sin(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.to_radians().sin().into())
    }

    /// https://ref.dm-lang.org/proc/cos
    ///
    /// Returns: The cosine of X, where X is in degrees.
    pub(crate) fn cos(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.to_radians().cos().into())
    }

    /// https://ref.dm-lang.org/proc/tan
    ///
    /// Returns: The tangent of X, where X is in degrees.
    pub(crate) fn tan(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.to_radians().tan().into())
    }

    /// https://ref.dm-lang.org/proc/arcsin
    ///
    /// Returns: The inverse sine of X in degrees.
    pub(crate) fn arcsin(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.asin().to_degrees().into())
    }

    /// https://ref.dm-lang.org/proc/arccos
    ///
    /// Returns: The inverse cosine of X in degrees.
    pub(crate) fn arccos(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.acos().to_degrees().into())
    }

    /// https://ref.dm-lang.org/proc/arctan
    ///
    /// Num: A number
    /// x, y: 2D coordinates
    /// Vector: A vector (only 2 dimensions are used)
    /// Returns: The inverse tangent in degrees.
    pub(crate) fn arctan(&self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let a = self.number(&arg(0))?;

        Ok(match args.len() {
            0 | 1 => a.atan().to_degrees(),
            _ => self.number(&arg(1))?.atan2(a).to_degrees(),
        }
        .into())
    }

    /// https://ref.dm-lang.org/proc/log
    ///
    /// Returns: The logarithm (base X) of Y. If X is not specified, a natural logarithm is computed (base 2.718…).
    pub(crate) fn log(&self, args: &[GenericValue]) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();

        let first = self.number(&arg(0))?;

        Ok(match args.len() {
            0 | 1 => first.ln(),
            _ => self.number(&arg(1))?.log(first),
        }
        .into())
    }

    /// https://ref.dm-lang.org/proc/sign
    ///
    /// Returns: 1 if A > 0, -1 if A < 0, 0 if A == 0
    pub(crate) fn sign(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(match self.number(value)? {
            n if n > 0.0 => 1.0,
            n if n < 0.0 => -1.0,
            _ => 0.0,
        }
        .into())
    }

    /// https://ref.dm-lang.org/proc/lerp
    ///
    /// Returns: An interpolated value between A and B
    pub(crate) fn lerp(&self, from: &GenericValue, to: &GenericValue, factor: &GenericValue) -> Result<GenericValue> {
        let from = self.number(from)?;
        let to = self.number(to)?;
        let factor = self.number(factor)?;

        Ok((from + (to - from) * factor).into())
    }

    /// https://ref.dm-lang.org/proc/fract
    ///
    /// Returns: fractional part of A
    pub(crate) fn fract(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.fract().into())
    }

    /// https://ref.dm-lang.org/proc/trunc
    ///
    /// Returns: truncated A
    pub(crate) fn trunc(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.trunc().into())
    }

    /// https://ref.dm-lang.org/proc/isnan
    ///
    /// n: A number
    pub(crate) fn isnan(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.is_nan().into())
    }

    /// https://ref.dm-lang.org/proc/isinf
    ///
    /// n: A number
    pub(crate) fn isinf(&self, value: &GenericValue) -> Result<GenericValue> {
        Ok(self.number(value)?.is_infinite().into())
    }

    /// https://ref.dm-lang.org/proc/alist
    ///
    /// Returns: A new associative list with contents (keys) A, B, C, and associated values a, b, c.
    pub(crate) fn dm_alist(&mut self, args: &[(Option<Identifier>, GenericValue)]) -> Result<GenericValue> {
        let entries = args
            .iter()
            .map(|(key, value)| match key {
                Some(key) => (GenericValue::Text(key.as_str().into()), Some(value.clone())),
                None => (value.clone(), None),
            })
            .collect::<Vec<_>>();

        self.alist(entries)
    }

    /// The file behind an `/icon` datum, or the value itself
    pub(crate) fn icon_file(&self, mut icon: GenericValue) -> GenericValue {
        for _ in 0..8 {
            let GenericValue::Object(id) = icon else {
                break;
            };
            icon = self
                .runtime
                .heap
                .object(id)
                .and_then(|object| object.vars.get(&Identifier::from(vars::ICON)))
                .cloned()
                .unwrap_or_default();
        }

        icon
    }

    pub(crate) fn construct(
        &mut self, path: &str, params: &[Identifier], target: Option<ObjectId>,
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let id = match target {
            Some(id) => id,
            None => {
                let ty = self
                    .tree
                    .id_of(&TreePath::parse(path))
                    .ok_or_else(|| self.fault(FaultKind::MissingVariable(path.into())))?;
                self.reserve(1)?;

                self.runtime
                    .heap
                    .alloc_object(Object::new(ty))
                    .map_err(|kind| self.fault(kind))?
            },
        };

        let mut positional = 0;
        for (key, value) in args {
            let key = match key {
                Some(key) => key,
                None => {
                    let Some(name) = params.get(positional).cloned() else {
                        break;
                    };
                    positional += 1;

                    name
                },
            };

            self.write_field(GenericValue::Object(id), key, value)?;
        }

        Ok(GenericValue::Object(id))
    }

    /// OpenDream's `HandleRange`. The arguments come in either order, and `"7x5"` spells a box.
    fn range_list(&mut self, args: [GenericValue; 2], include_center: bool, sight: Sight) -> Result<GenericValue> {
        let mut center = None;
        let mut size = (11, 11);
        for value in args {
            match value {
                GenericValue::Object(id) => center = Some(id),
                GenericValue::Num(distance) => {
                    let span = (distance.max(0.0) as i32).saturating_mul(2).saturating_add(1);
                    size = (span, span);
                },
                GenericValue::Text(text) => {
                    if let Some((width, height)) = text.split_once('x')
                        && let (Ok(width), Ok(height)) = (width.trim().parse::<i32>(), height.trim().parse::<i32>())
                    {
                        size = (width.max(0), height.max(0));
                    }
                },
                _ => {},
            }
        }

        let [world_width, world_height, _] = self.runtime.world.size;
        let size = (
            size.0.min(world_width.saturating_mul(2).saturating_add(1)),
            size.1.min(world_height.saturating_mul(2).saturating_add(1)),
        );
        let Some(center) = center else {
            return self.list(Vec::new());
        };

        // A bake caches against the 3 by 3 neighbourhood, which anything wider reaches past
        if size.0 > 3 || size.1 > 3 {
            self.is_cacheable = false;
        }

        let roots = self.tree.roots();
        let is = |evaluator: &Self, id: ObjectId, root: Option<objtree::TypeId>| {
            let ty = evaluator.runtime.heap.object(id).map(|object| object.ty);
            ty.zip(root)
                .is_some_and(|(ty, root)| evaluator.tree.is_subtype_of(ty, root))
        };

        let mut entries = Vec::new();
        let mut areas = HashSet::new();
        if is(self, center, roots.area) {
            self.is_cacheable = false;
            entries.push(GenericValue::Object(center));
            areas.insert(center);
            let mut positions = self
                .runtime
                .world
                .areas
                .iter()
                .filter(|(_, area)| **area == center)
                .map(|(position, _)| *position)
                .collect::<Vec<_>>();
            positions.sort_unstable_by_key(|position| (position.z, position.y, position.x));
            for position in positions {
                if let Some(turf) = self.runtime.world.turf_at(position) {
                    self.range_add_with_contents(&mut entries, &mut areas, turf, None)?;
                }
            }

            return self.range_finish(entries);
        }

        if is(self, center, roots.turf) {
            if include_center {
                self.range_add_with_contents(&mut entries, &mut areas, center, None)?;
            }
        } else {
            if include_center {
                for content in self.contents_of(center) {
                    self.range_add(&mut entries, &mut areas, content)?;
                }
            }

            let Some(loc) = self.runtime.heap.object(center).and_then(|object| object.loc) else {
                return self.range_finish(entries);
            };

            let skip = (!include_center).then_some(center);
            self.range_add_with_contents(&mut entries, &mut areas, loc, skip)?;
            if !is(self, loc, roots.turf) {
                return self.range_finish(entries);
            }
        }

        if let Some(origin) = self.runtime.world.position(&self.runtime.heap, center) {
            let tiles = dantom_spiral((origin.x, origin.y), size);
            self.charge(tiles.len())?;
            let visible = match sight {
                Sight::Through => None,
                Sight::Line => Some(self.visible_cells(origin, size)?),
            };

            for (x, y) in tiles {
                if visible.as_ref().is_some_and(|visible| !visible.contains(&(x, y))) {
                    continue;
                }

                if let Some(turf) = self.runtime.world.turf_at(Position::new(x, y, origin.z)) {
                    self.range_add_with_contents(&mut entries, &mut areas, turf, None)?;
                }
            }
        }

        self.range_finish(entries)
    }

    fn visible_cells(&mut self, origin: Position, size: (i32, i32)) -> Result<HashSet<(i32, i32)>> {
        let (low_x, low_y) = (origin.x - size.0 / 2, origin.y - size.1 / 2);
        let mut grid = ViewGrid::new(size.0, size.1);
        for grid_y in 0..size.1 {
            for grid_x in 0..size.0 {
                let position = Position::new(low_x + grid_x, low_y + grid_y, origin.z);
                let Some(turf) = self.runtime.world.turf_at(position) else {
                    continue;
                };

                let mut is_opaque = false;
                let mut cell = vec![turf];
                cell.extend(self.contents_of(turf));
                for id in cell {
                    if self
                        .read_field(GenericValue::Object(id), &Identifier::from(vars::OPACITY))?
                        .truthy()
                    {
                        is_opaque = true;
                        break;
                    }
                }

                grid.tiles[(grid_y * size.0 + grid_x) as usize] = Some(ViewTile {
                    is_opaque,
                    vis: 0,
                    vis2: 0,
                });
            }
        }

        grid.calculate();
        let mut visible = HashSet::new();
        for grid_y in 0..size.1 {
            for grid_x in 0..size.0 {
                if grid.get(grid_x, grid_y).is_some_and(|tile| tile.vis != 0) {
                    visible.insert((low_x + grid_x, low_y + grid_y));
                }
            }
        }

        Ok(visible)
    }

    fn range_finish(&mut self, entries: Vec<GenericValue>) -> Result<GenericValue> {
        self.charge(entries.len())?;

        self.list(entries.into_iter().map(|entry| (entry, None)).collect())
    }

    fn contents_of(&self, id: ObjectId) -> Vec<ObjectId> {
        self.runtime
            .heap
            .object(id)
            .map(|object| object.contents.clone())
            .unwrap_or_default()
    }

    pub(crate) fn range_add_with_contents(
        &mut self, entries: &mut Vec<GenericValue>, areas: &mut HashSet<ObjectId>, id: ObjectId, skip: Option<ObjectId>,
    ) -> Result<()> {
        self.range_add(entries, areas, id)?;
        for content in self.contents_of(id) {
            if Some(content) != skip {
                self.range_add(entries, areas, content)?;
            }
        }

        Ok(())
    }

    /// `invisibility = 101` hides an atom from every range, and a turf brings its area in once
    pub(crate) fn range_add(
        &mut self, entries: &mut Vec<GenericValue>, areas: &mut HashSet<ObjectId>, id: ObjectId,
    ) -> Result<()> {
        let invisibility = self.read_field(GenericValue::Object(id), &Identifier::from(vars::INVISIBILITY))?;
        if invisibility.num().is_some_and(|value| value >= 101.0) {
            return Ok(());
        }

        entries.push(GenericValue::Object(id));
        let turf = self.tree.roots().turf;
        let is_turf = self
            .runtime
            .heap
            .object(id)
            .zip(turf)
            .is_some_and(|(object, turf)| self.tree.is_subtype_of(object.ty, turf));
        if is_turf
            && let Some(area) = self.runtime.world.area_of(&self.runtime.heap, id)
            && areas.insert(area)
        {
            entries.push(GenericValue::Object(area));
        }

        Ok(())
    }

    pub(crate) fn resource_path(&mut self, file: &GenericValue) -> Result<Option<PathBuf>> {
        let Some(name) = file.text() else {
            return Ok(None);
        };
        let Some(root) = self.runtime.world.root.clone() else {
            return Err(self.fault(FaultKind::Blocked("filesystem".into())));
        };

        file_path::within(&root, name)
            .map(Some)
            .ok_or_else(|| self.fault(FaultKind::Blocked("filesystem".into())))
    }
}

pub(crate) fn decode_param(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => match rest
                .first_chunk::<2>()
                .and_then(|hex| u8::from_str_radix(str::from_utf8(hex).ok()?, 16).ok())
            {
                Some(decoded) => {
                    bytes.push(decoded);
                    rest = &rest[2..];
                },
                None => bytes.push(byte),
            },
            _ => bytes.push(byte),
        }
    }

    String::from_utf8_lossy(&bytes).into_owned()
}
