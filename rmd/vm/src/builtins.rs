use core::{path::TreePath, types::Identifier};

use objtree::TypeId;

use crate::{
    Fault,
    FaultKind,
    GenericValue,
    Intrinsic,
    eval::Evaluator,
    heap::ObjectId,
    value::{ListId, Receiver},
};

type Result<T> = std::result::Result<T, Fault>;

impl Evaluator<'_> {
    pub fn builtin(&mut self, name: &str, args: Vec<GenericValue>, src: Option<ObjectId>) -> Result<GenericValue> {
        let arg = |n| args.get(n).cloned().unwrap_or_default();
        match name {
            "call" | "call_ext" => {
                let (src, ty, name) = match arg(0) {
                    GenericValue::Object(id) => (
                        Some(id),
                        self.runtime.heap.object(id).map(|o| o.ty),
                        arg(1).text().map(Identifier::from),
                    ),
                    GenericValue::Path(path) => {
                        let owner = TreePath::new(path.declaration_owner().to_vec(), true);
                        (
                            if owner.segments.is_empty() { None } else { src },
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
            _ => {
                let Some(intrinsic) = crate::Intrinsic::from_name(name) else {
                    return Err(self.fault(FaultKind::MissingProc(name.into())));
                };

                self.intrinsic_impl(
                    intrinsic,
                    Receiver::None,
                    &[],
                    args.into_iter().map(|value| (None, value)).collect(),
                )
            },
        }
    }

    pub(crate) fn types_of(&mut self, args: Vec<GenericValue>, include_self: bool) -> Result<GenericValue> {
        let mut entries = Vec::new();
        for value in args {
            if let GenericValue::Path(path) = value
                && let Some(ty) = self.tree.id_of(&path)
            {
                let descendants = self.tree.descendants(ty);
                self.reserve(descendants.len())?;
                for id in descendants {
                    if !include_self && id == ty {
                        continue;
                    }

                    if let Some(decl) = self.tree.get(id) {
                        entries.push((GenericValue::Path(decl.path.clone()), None));
                    }
                }
            }
        }

        self.list(entries)
    }

    pub(crate) fn is_root_type(&self, value: &GenericValue, root: Option<TypeId>) -> bool {
        value
            .object()
            .and_then(|id| self.runtime.heap.object(id))
            .is_some_and(|o| root.is_some_and(|r| self.tree.is_subtype_of(o.ty, r)))
    }

    pub fn appearance_object(
        &mut self, name: &str, target: Option<ObjectId>, params: &[Identifier],
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let path = if name == "image" {
            "/image"
        } else {
            "/mutable_appearance"
        };

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
                    .alloc_object(crate::heap::Object::new(ty))
                    .map_err(|e| self.fault(e))?
            },
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

                let key = params
                    .get(positional)
                    .cloned()
                    .ok_or_else(|| self.fault(FaultKind::InvalidOperation("appearance arguments".into())))?;
                positional += 1;
                key
            };

            if key.as_str() == "icon" && matches!(value, GenericValue::Object(_)) {
                self.write_field(GenericValue::Object(id), "appearance".into(), value)?;
            } else {
                self.write_field(GenericValue::Object(id), key, value)?;
            }
        }

        Ok(GenericValue::Object(id))
    }

    pub fn list_builtin(&mut self, proc: Intrinsic, id: ListId, args: Vec<GenericValue>) -> Result<GenericValue> {
        let mut entries = self
            .runtime
            .heap
            .list(id)
            .map(|l| l.entries.clone())
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        self.charge(entries.len().max(1))?;

        let len = entries.len();
        let mut result = GenericValue::Null;
        match proc {
            Intrinsic::ListAdd | Intrinsic::ListRemove => {
                let mut removed = false;
                for arg in args {
                    let values = if matches!(arg, GenericValue::List(_) | GenericValue::ArgList(_)) {
                        self.iter_values(arg)?
                    } else {
                        vec![(arg, None)]
                    };
                    self.charge(entries.len().saturating_mul(values.len()).max(1))?;
                    for (value, assoc) in values {
                        if proc == Intrinsic::ListAdd {
                            self.reserve(1)?;
                            entries.push((value, assoc));
                        } else if let Some(index) = entries.iter().position(|(v, _)| *v == value) {
                            entries.remove(index);
                            removed = true;
                        }
                    }
                }
                if proc == Intrinsic::ListRemove {
                    result = removed.into();
                }
            },
            Intrinsic::ListRemoveAll => {
                let mut removed = 0usize;
                for arg in args {
                    let values = if matches!(arg, GenericValue::List(_) | GenericValue::ArgList(_)) {
                        self.iter_values(arg)?
                    } else {
                        vec![(arg, None)]
                    };
                    self.charge(entries.len().saturating_mul(values.len()).max(1))?;
                    for (value, _) in values {
                        let before = entries.len();
                        entries.retain(|(entry, _)| *entry != value);
                        removed += before - entries.len();
                    }
                }

                result = removed.into();
            },
            Intrinsic::ListSplice => {
                let start = index_arg(&args, 0, 1, len + 1);
                let end = index_arg(&args, 1, 0, len + 1);
                let mut inserted = Vec::new();
                for arg in args.into_iter().skip(2) {
                    let values = if matches!(arg, GenericValue::List(_) | GenericValue::ArgList(_)) {
                        self.iter_values(arg)?
                    } else {
                        vec![(arg, None)]
                    };
                    inserted.extend(values);
                }
                self.reserve(inserted.len())?;
                drop(entries.splice(start.min(end)..end, inserted));
            },
            Intrinsic::ListFind => {
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
            Intrinsic::ListCopy | Intrinsic::ListCut => {
                let start = index_arg(&args, 0, 1, len + 1);
                let end = index_arg(&args, 1, 0, len + 1);
                if proc == Intrinsic::ListCopy {
                    return self.list(entries.get(start.min(end)..end).unwrap_or_default().to_vec());
                }
                entries.drain(start.min(end)..end);
            },
            Intrinsic::ListInsert => {
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
            Intrinsic::ListJoin => {
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
            Intrinsic::ListSwap => {
                let a = index_arg(&args, 0, 1, len + 1);
                let b = index_arg(&args, 1, 1, len + 1);
                if a >= len || b >= len {
                    return Err(self.fault(FaultKind::InvalidOperation("list index out of bounds".into())));
                }
                entries.swap(a, b);
            },
            _ => return Err(self.fault(FaultKind::Unsupported(format!("{} as a list proc", proc.name())))),
        }

        self.list_mut(id)?.replace(entries);

        Ok(result)
    }
}

pub(crate) fn index_arg(args: &[GenericValue], index: usize, default: i32, end: usize) -> usize {
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
