use core::{path::TreePath, types::Identifier};

use crate::{
    Fault,
    FaultKind,
    GenericValue,
    eval::{Evaluator, Frame},
    value::ListId,
    world::Position,
};

type Result<T> = std::result::Result<T, Fault>;

pub(crate) fn blocked(name: &str) -> bool {
    name.starts_with("rustg_")
        || matches!(
            name,
            "file"
                | "fcopy"
                | "fcopy_rsc"
                | "fdel"
                | "flist"
                | "fexists"
                | "file2text"
                | "text2file"
                | "shell"
                | "sleep"
                | "spawn"
                | "addtimer"
                | "input"
                | "winset"
                | "winget"
                | "browse"
                | "browse_rsc"
                | "link"
                | "ftp"
                | "shutdown"
                | "reboot"
                | "savefile"
                | "database"
        )
}

impl Evaluator<'_> {
    pub fn builtin(&mut self, name: &str, args: Vec<GenericValue>, frame: &mut Frame) -> Result<GenericValue> {
        if blocked(name) {
            return Err(self.fault(FaultKind::Blocked(name.into())));
        }

        if name == "locate" && (args.len() >= 3 || args.len() < 2) {
            self.memo_safe = false;
        }

        let arg = |n| args.get(n).cloned().unwrap_or_default();
        let text = |n| args.get(n).and_then(GenericValue::text).unwrap_or_default();
        let number = |n| self.number(&arg(n));
        match name {
            "call" => {
                let (src, ty, name) = match arg(0) {
                    GenericValue::Object(id) => (
                        Some(id),
                        self.runtime.heap.object(id).map(|o| o.ty),
                        arg(1).text().map(Identifier::from),
                    ),
                    GenericValue::Path(path) => {
                        let owner = TreePath::new(path.declaration_owner().to_vec(), true);
                        (
                            if owner.segments.is_empty() { None } else { frame.src },
                            self.tree.id_of(&owner),
                            path.name().cloned(),
                        )
                    },
                    GenericValue::Text(name) if args.len() == 1 => {
                        (None, Some(objtree::TypeId::ROOT), Some(name.as_ref().into()))
                    },
                    _ => return Err(self.fault(FaultKind::Blocked("external call".into()))),
                };
                let proc = ty
                    .zip(name)
                    .and_then(|(ty, name)| self.find_proc(ty, &name))
                    .ok_or_else(|| self.fault(FaultKind::MissingProc("dynamic DM proc".into())))?;
                Ok(GenericValue::Proc(crate::value::ProcRef { src, proc }))
            },
            "to_chat" | "stack_trace" => Ok(GenericValue::Null),
            "isnull" => Ok(matches!(arg(0), GenericValue::Null).into()),
            "isnum" => Ok(matches!(arg(0), GenericValue::Num(_)).into()),
            "istext" => Ok(matches!(arg(0), GenericValue::Text(_)).into()),
            "islist" => Ok(matches!(arg(0), GenericValue::List(_) | GenericValue::ArgList(_)).into()),
            "isicon" => Ok(match arg(0) {
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
            .into()),
            "ispath" | "istype" => {
                let ty = match arg(0) {
                    GenericValue::Object(id) if name == "istype" => self.runtime.heap.object(id).map(|o| o.ty),
                    GenericValue::Path(path) => self.tree.id_of(&path),
                    _ => None,
                };
                let base = match arg(1) {
                    GenericValue::Path(path) => self.tree.id_of(&path),
                    _ => None,
                };
                Ok(ty
                    .is_some_and(|ty| base.is_none_or(|base| self.tree.is_subtype_of(ty, base)))
                    .into())
            },
            "isturf" | "isarea" | "ismovable" | "isobj" | "ismob" | "isatom" | "isloc" => {
                let roots = self.tree.roots();
                let root = match name {
                    "isturf" => roots.turf,
                    "isarea" => roots.area,
                    "ismovable" => roots.movable,
                    "isobj" => roots.obj,
                    "ismob" => roots.mob,
                    _ => roots.atom,
                };
                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.heap.object(id))
                    .is_some_and(|o| root.is_some_and(|r| self.tree.is_subtype_of(o.ty, r)))
                    .into())
            },
            "length" | "length_char" => Ok((match arg(0) {
                GenericValue::List(id) | GenericValue::ArgList(id) => {
                    self.runtime.heap.list(id).map_or(0, |l| l.entries.len())
                },
                GenericValue::Text(s) | GenericValue::Resource(s) => s.chars().count(),
                GenericValue::Null => 0,
                GenericValue::Object(id) => self.runtime.heap.object(id).map_or(0, |o| o.contents.len()),
                _ => 0,
            } as f32)
                .into()),
            "typesof" | "subtypesof" => {
                let mut entries = Vec::new();
                for value in args {
                    if let GenericValue::Path(path) = value
                        && let Some(ty) = self.tree.id_of(&path)
                    {
                        let descendants = self.tree.descendants(ty);
                        self.reserve(descendants.len())?;
                        for id in descendants {
                            if name == "subtypesof" && id == ty {
                                continue;
                            }

                            if let Some(decl) = self.tree.get(id) {
                                entries.push((GenericValue::Path(decl.path.clone()), None));
                            }
                        }
                    }
                }
                self.list(entries)
            },
            "text2path" => {
                let path = TreePath::parse(text(0));
                Ok(self
                    .tree
                    .id_of(&path)
                    .map(|_| GenericValue::Path(path))
                    .unwrap_or_default())
            },
            "min" | "max" => {
                let values = if args.len() == 1 && matches!(arg(0), GenericValue::List(_) | GenericValue::ArgList(_)) {
                    self.iter_values(arg(0))?.into_iter().map(|(v, _)| v).collect()
                } else {
                    args
                };
                let mut result: Option<f32> = None;
                for value in values {
                    let n = self.number(&value)?;
                    result = Some(result.map_or(n, |r| if name == "min" { r.min(n) } else { r.max(n) }));
                }
                Ok(result.unwrap_or(0.0).into())
            },
            "abs" => Ok(number(0)?.abs().into()),
            "ceil" => Ok(number(0)?.ceil().into()),
            "floor" => Ok(number(0)?.floor().into()),
            "sqrt" => Ok(number(0)?.sqrt().into()),
            "round" => {
                let n = number(0)?;
                if args.len() < 2 {
                    Ok(n.floor().into())
                } else {
                    let step = number(1)?;
                    Ok(if step == 0.0 {
                        n
                    } else {
                        (n / step + 0.5).floor() * step
                    }
                    .into())
                }
            },
            "clamp" => {
                let n = number(0)?;
                let lo = number(1)?;
                let hi = number(2)?;
                if lo.is_nan() || hi.is_nan() || lo > hi {
                    return Err(self.fault(FaultKind::InvalidOperation("invalid clamp bounds".into())));
                }
                Ok(n.clamp(lo, hi).into())
            },
            "rand" => {
                let (lo, hi) = match args.len() {
                    0 => (0.0, 1.0),
                    1 => (0.0, number(0)?),
                    _ => (number(0)?, number(1)?),
                };
                let r = self.random();
                Ok(if args.is_empty() {
                    r
                } else {
                    lo.min(hi) + (r * ((hi - lo).abs() + 1.0)).floor()
                }
                .into())
            },
            "prob" => {
                let n = number(0)?;
                Ok((self.random() * 100.0 < n).into())
            },
            "get_step" => {
                let dir = number(1)? as i32;
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
                    self.memo_safe = false;
                }
                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.world.get_step(&self.runtime.heap, id, dir))
                    .map(GenericValue::Object)
                    .unwrap_or_default())
            },
            "get_dir" | "get_dist" => {
                self.position_sensitive = true;
                let pos = |v: GenericValue| {
                    v.object()
                        .and_then(|id| self.runtime.world.position(&self.runtime.heap, id))
                };
                let (Some(a), Some(b)) = (pos(arg(0)), pos(arg(1))) else {
                    return Ok(0.0.into());
                };
                if name == "get_dir" {
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
            },
            "get_turf" => Ok(arg(0)
                .object()
                .and_then(|id| self.runtime.world.position(&self.runtime.heap, id))
                .and_then(|p| self.runtime.world.turf_at(p))
                .map(GenericValue::Object)
                .unwrap_or_default()),
            "get_area" => Ok(arg(0)
                .object()
                .and_then(|id| self.runtime.world.area_of(&self.runtime.heap, id))
                .map(GenericValue::Object)
                .unwrap_or_default()),
            "locate" => {
                if args.len() >= 3 {
                    let pos = Position::new(number(0)? as i32, number(1)? as i32, number(2)? as i32);
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
            },
            "turn" => {
                let dirs = [1, 9, 8, 10, 2, 6, 4, 5];
                let dir = number(0)? as i32;
                let angle = number(1)?;
                let Some(index) = dirs.iter().position(|d| *d == dir) else {
                    return Ok(dir.into());
                };
                let index = (index as i32 + (angle.rem_euclid(360.0) / 45.0).round() as i32).rem_euclid(8) as usize;
                Ok(dirs.get(index).copied().unwrap_or(0).into())
            },
            // `rgb(r, g, b)` and `rgb(r, g, b, a)` over 0..255. The named and `space` forms are not
            // handled, so anything that is not three or four numbers stays a fault.
            "rgb" if (3..=4).contains(&args.len()) && args.iter().all(|a| matches!(a, GenericValue::Num(_))) => {
                let mut color = String::from("#");
                for index in 0..args.len() {
                    let channel = number(index)?.round().clamp(0.0, 255.0) as u8;
                    color.push_str(&format!("{channel:02x}"));
                }

                self.text(color)
            },
            "num2text" => self.text(number(0)?.to_string()),
            "text2num" => Ok(text(0).trim().parse::<f32>().map(GenericValue::Num).unwrap_or_default()),
            "uppertext" => self.text(text(0).to_uppercase()),
            "lowertext" => self.text(text(0).to_lowercase()),
            "sorttext" | "sorttextEx" => {
                self.charge(text(0).len().saturating_add(text(1).len()))?;
                let order = if name == "sorttextEx" {
                    text(1).cmp(text(0))
                } else {
                    text(1).to_lowercase().cmp(&text(0).to_lowercase())
                };
                Ok((order as i32).into())
            },
            "copytext" | "copytext_char" => {
                let s = text(0);
                let chars = s.chars().collect::<Vec<_>>();
                self.charge(chars.len())?;
                let start = index_arg(&args, 1, 1, chars.len() + 1);
                let end = index_arg(&args, 2, 0, chars.len() + 1);
                self.text(chars.get(start.min(end)..end).unwrap_or_default().iter().collect())
            },
            "findtext" | "findtextEx" | "findlasttext" | "findlasttextEx" => {
                let case = name.ends_with("Ex");
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
                let start = index_arg(&args, 2, 1, chars.len() + 1);
                let end = index_arg(&args, 3, 0, chars.len() + 1);
                let segment: String = chars.get(start.min(end)..end).unwrap_or_default().iter().collect();
                let found = if name.starts_with("findlast") {
                    segment.rfind(&needle)
                } else {
                    segment.find(&needle)
                };
                Ok(found
                    .map(|offset| start + segment.get(..offset).unwrap_or_default().chars().count() + 1)
                    .unwrap_or(0)
                    .into())
            },
            "replacetext" | "replacetextEx" => {
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
                        if name == "replacetextEx" {
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
            },
            "splittext" => {
                let source = text(0);
                let delimiter = text(1);
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
            },
            "json_decode" => {
                let value = crate::json::decode(
                    text(0),
                    self.limits.allocations.min(self.instruction_budget_remaining as usize),
                )
                .map_err(|e| self.fault(e))?;
                self.reserve(text(0).len())?;
                self.constant(&value)
            },
            "ref" => {
                self.memo_safe = false;
                self.text(arg(0).display())
            },
            "flick" | "animate" => Err(self.fault(FaultKind::Blocked(name.into()))),
            "hascall" => {
                let name = Identifier::from(text(1));
                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.heap.object(id))
                    .and_then(|o| self.find_proc(o.ty, &name))
                    .is_some()
                    .into())
            },
            "CRASH" => Err(self.fault(FaultKind::InvalidOperation(arg(0).display()))),
            _ => {
                let _ = frame;
                Err(self.fault(FaultKind::MissingProc(name.into())))
            },
        }
    }

    pub fn appearance_object(
        &mut self, name: &str, args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let path = if name == "image" {
            "/image"
        } else {
            "/mutable_appearance"
        };

        let ty = self
            .tree
            .id_of(&TreePath::parse(path))
            .ok_or_else(|| self.fault(FaultKind::MissingVariable(path.into())))?;
        self.reserve(1)?;

        let id = self
            .runtime
            .heap
            .alloc_object(crate::heap::Object::new(ty))
            .map_err(|e| self.fault(e))?;

        let names = if name == "image" {
            vec!["icon", "loc", "icon_state", "layer", "dir", "pixel_x", "pixel_y"]
        } else {
            vec!["icon", "icon_state", "layer", "plane", "dir", "alpha"]
        };

        let mut positional = 0;
        for (key, value) in args {
            let key = if let Some(key) = key {
                key
            } else {
                if positional == 1 && name == "image" && !matches!(value, GenericValue::Object(_) | GenericValue::Null)
                {
                    positional += 1;
                }

                let key = names
                    .get(positional)
                    .copied()
                    .ok_or_else(|| self.fault(FaultKind::InvalidOperation("appearance arguments".into())))?;
                positional += 1;
                Identifier::from(key)
            };

            if key.as_str() == "icon" && matches!(value, GenericValue::Object(_)) {
                self.write_field(GenericValue::Object(id), "appearance".into(), value)?;
            } else {
                self.write_field(GenericValue::Object(id), key, value)?;
            }
        }

        Ok(GenericValue::Object(id))
    }

    pub fn list_builtin(&mut self, id: ListId, name: &str, args: Vec<GenericValue>) -> Result<GenericValue> {
        let mut entries = self
            .runtime
            .heap
            .list(id)
            .map(|l| l.entries.clone())
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        self.charge(entries.len().max(1))?;

        let len = entries.len();
        let mut result = GenericValue::Null;
        match name {
            "Add" | "Remove" => {
                let mut removed = false;
                for arg in args {
                    let values = if matches!(arg, GenericValue::List(_) | GenericValue::ArgList(_)) {
                        self.iter_values(arg)?
                    } else {
                        vec![(arg, None)]
                    };
                    self.charge(entries.len().saturating_mul(values.len()).max(1))?;
                    for (value, assoc) in values {
                        if name == "Add" {
                            self.reserve(1)?;
                            entries.push((value, assoc));
                        } else if let Some(index) = entries.iter().position(|(v, _)| *v == value) {
                            entries.remove(index);
                            removed = true;
                        }
                    }
                }
                if name == "Remove" {
                    result = removed.into();
                }
            },
            "Find" => {
                let value = args.first().cloned().unwrap_or_default();
                let start = index_arg(&args, 1, 1, len + 1);
                let end = index_arg(&args, 2, 0, len + 1);
                return Ok(entries
                    .get(start.min(end)..end)
                    .unwrap_or_default()
                    .iter()
                    .position(|(v, _)| *v == value)
                    .map(|i| i + start + 1)
                    .unwrap_or(0)
                    .into());
            },
            "Copy" | "Cut" => {
                let start = index_arg(&args, 0, 1, len + 1);
                let end = index_arg(&args, 1, 0, len + 1);
                if name == "Copy" {
                    return self.list(entries.get(start.min(end)..end).unwrap_or_default().to_vec());
                }
                entries.drain(start.min(end)..end);
            },
            "Insert" => {
                let mut index = index_arg(&args, 0, 1, len + 1);
                for arg in args.into_iter().skip(1) {
                    let values = if matches!(arg, GenericValue::List(_) | GenericValue::ArgList(_)) {
                        self.iter_values(arg)?
                    } else {
                        vec![(arg, None)]
                    };
                    self.reserve(values.len())?;
                    for entry in values {
                        entries.insert(index.min(entries.len()), entry);
                        index += 1;
                    }
                }
            },
            "Join" => {
                let delimiter = args.first().and_then(GenericValue::text).unwrap_or_default();
                let start = index_arg(&args, 1, 1, len + 1);
                let end = index_arg(&args, 2, 0, len + 1);
                let mut output = String::new();
                for (index, (v, _)) in entries.get(start.min(end)..end).unwrap_or_default().iter().enumerate() {
                    let value = v.display();
                    let extra = value.len() + if index == 0 { 0 } else { delimiter.len() };
                    if output.len().saturating_add(extra) > self.limits.text_bytes {
                        return Err(self.fault(FaultKind::Memory));
                    }
                    if index > 0 {
                        output.push_str(delimiter);
                    }
                    output.push_str(&value);
                }
                return self.text(output);
            },
            "Swap" => {
                let a = index_arg(&args, 0, 1, len + 1);
                let b = index_arg(&args, 1, 1, len + 1);
                if a >= len || b >= len {
                    return Err(self.fault(FaultKind::InvalidOperation("list index out of bounds".into())));
                }
                entries.swap(a, b);
            },
            _ => return Err(self.fault(FaultKind::MissingProc(format!("list.{name}")))),
        }

        self.list_mut(id)?.replace(entries);

        Ok(result)
    }
}

fn index_arg(args: &[GenericValue], index: usize, default: i32, end: usize) -> usize {
    let n = args
        .get(index)
        .and_then(GenericValue::num)
        .map(|n| n as i32)
        .unwrap_or(default);

    let n = if n < 0 {
        end as i64 + n as i64
    } else if n == 0 {
        end as i64
    } else {
        n as i64
    };

    (n.saturating_sub(1).max(0) as usize).min(end.saturating_sub(1))
}
