use core::{path::TreePath, types::Identifier};
use std::{collections::HashSet, path::PathBuf};

use crate::{
    FaultKind,
    GenericValue,
    Intrinsic,
    builtins::index_arg,
    eval::Evaluator,
    heap::ObjectId,
    value::{ListData, Receiver},
    world::Position,
};

type Result<T> = std::result::Result<T, crate::Fault>;
impl Evaluator<'_> {
    pub(crate) fn intrinsic_impl(
        &mut self, intrinsic: Intrinsic, receiver: Receiver, name: &str, params: &[Identifier],
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).map(|(_, value)| value.clone()).unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|(_, value)| value.text()).unwrap_or_default();
        let positional = || args.iter().map(|(_, value)| value.clone()).collect::<Vec<_>>();

        match intrinsic {
            Intrinsic::Call | Intrinsic::CallExt => {
                let (src, ty, proc_name) = match arg(0) {
                    GenericValue::Object(id) => (
                        Receiver::Object(id),
                        self.runtime.heap.object(id).map(|object| object.ty),
                        arg(1).text().map(Identifier::from),
                    ),
                    GenericValue::List(id) | GenericValue::ArgList(id) => {
                        let receiver = Receiver::List(id);
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
                                Receiver::None
                            } else {
                                self.current_receiver
                            },
                            self.tree.id_of(&owner),
                            path.name().cloned(),
                        )
                    },
                    GenericValue::Text(proc_name) if args.len() == 1 => (
                        Receiver::None,
                        Some(objtree::TypeId::ROOT),
                        Some(proc_name.as_ref().into()),
                    ),
                    _ => return Err(self.fault(FaultKind::Blocked("external call".into()))),
                };
                let proc = ty
                    .zip(proc_name)
                    .and_then(|(ty, proc_name)| self.find_proc(ty, &proc_name))
                    .ok_or_else(|| self.fault(FaultKind::MissingProc("dynamic DM proc".into())))?;
                Ok(GenericValue::Proc(crate::value::ProcRef { src, proc }))
            },
            Intrinsic::Arglist => match arg(0) {
                GenericValue::List(id) | GenericValue::ArgList(id) => Ok(GenericValue::ArgList(id)),
                _ => Err(self.fault(FaultKind::InvalidReference)),
            },

            Intrinsic::WorldNew | Intrinsic::WorldDel | Intrinsic::WorldTopic | Intrinsic::WorldError => {
                Ok(GenericValue::Null)
            },

            Intrinsic::WorldIsSubscribed
            | Intrinsic::WorldGetScores
            | Intrinsic::WorldGetMedal
            | Intrinsic::WorldGetCredits => Ok(GenericValue::Null),
            Intrinsic::WorldIsBanned => Ok(false.into()),
            Intrinsic::WorldAddCredits | Intrinsic::WorldPayCredits => Ok(0.into()),
            Intrinsic::WorldSetScores | Intrinsic::WorldSetMedal | Intrinsic::WorldClearMedal => Ok(GenericValue::Null),
            Intrinsic::WorldProfile | Intrinsic::WorldGetConfig | Intrinsic::WorldSetConfig => Ok(GenericValue::Null),

            Intrinsic::WorldReboot
            | Intrinsic::WorldRepop
            | Intrinsic::WorldExport
            | Intrinsic::WorldImport
            | Intrinsic::WorldOpenPort => Err(self.fault(FaultKind::Blocked(name.into()))),

            Intrinsic::WorldFile2List => self.file2list(&arg(0), &arg(1)),

            Intrinsic::DatumNew
            | Intrinsic::DatumDel
            | Intrinsic::DatumTopic
            | Intrinsic::AtomClick
            | Intrinsic::AtomDblClick
            | Intrinsic::AtomMouseDown
            | Intrinsic::AtomMouseDrag
            | Intrinsic::AtomMouseDrop
            | Intrinsic::AtomMouseEntered
            | Intrinsic::AtomMouseExited
            | Intrinsic::AtomMouseMove
            | Intrinsic::AtomMouseUp
            | Intrinsic::AtomMouseWheel
            | Intrinsic::AtomEntered
            | Intrinsic::AtomExited
            | Intrinsic::AtomCrossed
            | Intrinsic::AtomUncrossed
            | Intrinsic::AtomStat
            | Intrinsic::MovableBump => Ok(GenericValue::Null),

            Intrinsic::DmNewIcon => Ok(arg(0)),
            Intrinsic::Range | Intrinsic::Orange => self.range_list([arg(0), arg(1)], intrinsic == Intrinsic::Range),
            Intrinsic::IconStates => {
                let mut icon = arg(0);
                for _ in 0..8 {
                    let GenericValue::Object(id) = icon else {
                        break;
                    };
                    icon = self
                        .runtime
                        .heap
                        .object(id)
                        .and_then(|object| object.vars.get(&Identifier::from("icon")))
                        .cloned()
                        .unwrap_or_default();
                }

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
            },

            Intrinsic::DatumRead
            | Intrinsic::DatumWrite
            | Intrinsic::MovableMove
            | Intrinsic::MobLogin
            | Intrinsic::MobLogout
            | Intrinsic::ClientNew
            | Intrinsic::ClientDel
            | Intrinsic::ClientTopic
            | Intrinsic::ClientStat
            | Intrinsic::ClientCommand
            | Intrinsic::ClientImport
            | Intrinsic::ClientExport
            | Intrinsic::ClientAllowUpload
            | Intrinsic::ClientSoundQuery
            | Intrinsic::ClientMeasureText
            | Intrinsic::ClientMove
            | Intrinsic::ClientClick
            | Intrinsic::ClientDblClick
            | Intrinsic::ClientMouseDown
            | Intrinsic::ClientMouseDrag
            | Intrinsic::ClientMouseDrop
            | Intrinsic::ClientMouseEntered
            | Intrinsic::ClientMouseExited
            | Intrinsic::ClientMouseMove
            | Intrinsic::ClientMouseUp
            | Intrinsic::ClientMouseWheel
            | Intrinsic::ClientIsByondMember
            | Intrinsic::ClientCheckPassport
            | Intrinsic::ClientSendPage
            | Intrinsic::ClientGetAPI
            | Intrinsic::ClientSetAPI
            | Intrinsic::ClientRenderIcon
            | Intrinsic::SavefileNew
            | Intrinsic::SavefileFlush
            | Intrinsic::SavefileExportText
            | Intrinsic::SavefileImportText
            | Intrinsic::SavefileLock
            | Intrinsic::SavefileUnlock
            | Intrinsic::VectorNew
            | Intrinsic::VectorCross
            | Intrinsic::VectorDot
            | Intrinsic::VectorInterpolate
            | Intrinsic::VectorNormalize
            | Intrinsic::VectorTurn
            | Intrinsic::PixlocNew
            | Intrinsic::DmTurnIcon
            | Intrinsic::DmFlipIcon
            | Intrinsic::DmShiftIcon
            | Intrinsic::DmIconIntensity
            | Intrinsic::DmIconBlend
            | Intrinsic::DmIconSwapColor
            | Intrinsic::DmIconDrawBox
            | Intrinsic::DmIconInsert
            | Intrinsic::DmIconMapColors
            | Intrinsic::DmIconScale
            | Intrinsic::DmIconCrop
            | Intrinsic::DmIconGetPixel
            | Intrinsic::DmIconSize
            | Intrinsic::DmDatabase
            | Intrinsic::MakeGenerator => Err(self.fault(FaultKind::Blocked(name.into()))),

            Intrinsic::DemirProfile => Ok(self.runtime.profile.map(GenericValue::Object).unwrap_or_default()),

            Intrinsic::DemirNodeGroup => {
                if !self.runtime.defining_groups {
                    return Err(self.fault(FaultKind::Blocked(format!("{name} outside a profile's New()"))));
                }

                let Some(subtype) = args.first().and_then(|(_, value)| match value {
                    GenericValue::Path(path) => self.tree.id_of(path),
                    GenericValue::Object(id) => self.runtime.heap.object(*id).map(|object| object.ty),
                    _ => None,
                }) else {
                    return Ok(GenericValue::Null);
                };
                let blocker_arg = arg(1);
                let blocker_values = match blocker_arg {
                    GenericValue::Null => Vec::new(),
                    value @ (GenericValue::List(_) | GenericValue::ArgList(_)) => {
                        self.iter_values(value)?.into_iter().map(|(value, _)| value).collect()
                    },
                    value => vec![value],
                };
                let resolve_type = |value: &GenericValue| match value {
                    GenericValue::Path(path) => self.tree.id_of(path),
                    GenericValue::Object(id) => self.runtime.heap.object(*id).map(|object| object.ty),
                    _ => None,
                };
                let Some(blockers) = blocker_values.iter().map(resolve_type).collect::<Option<Vec<_>>>() else {
                    return Ok(GenericValue::Null);
                };

                if let Some(group) = self
                    .runtime
                    .node_groups
                    .iter_mut()
                    .find(|group| group.subtype == subtype)
                {
                    for blocker in blockers {
                        if !group.blockers.contains(&blocker) {
                            group.blockers.push(blocker);
                        }
                    }
                } else {
                    self.runtime.node_groups.push(crate::bake::NodeGroup {
                        subtype,
                        blockers: blockers.into_iter().fold(Vec::new(), |mut unique, blocker| {
                            if !unique.contains(&blocker) {
                                unique.push(blocker);
                            }
                            unique
                        }),
                    });
                }

                Ok(GenericValue::Null)
            },

            Intrinsic::DemirDefineGroup => {
                if !self.runtime.defining_groups {
                    return Err(self.fault(FaultKind::Blocked(format!("{name} outside a profile's New()"))));
                }

                let group = args.first().map(|(_, value)| value.clone()).unwrap_or_default().num();
                let group = group
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .unwrap_or_default() as u32;
                let ty = match args.get(1).map(|(_, value)| value.clone()) {
                    Some(GenericValue::Path(path)) => self.tree.id_of(&path),
                    Some(GenericValue::Object(id)) => self.runtime.heap.object(id).map(|object| object.ty),
                    _ => None,
                };
                if let Some(ty) = ty {
                    *self.runtime.groups.entry(ty).or_default() |= group;
                }

                Ok(GenericValue::Null)
            },

            Intrinsic::DemirRebake
            | Intrinsic::ImguiDockspace
            | Intrinsic::ImguiSetNextWindowDock
            | Intrinsic::ImguiSetNextWindowSize
            | Intrinsic::ImguiBegin
            | Intrinsic::ImguiEnd
            | Intrinsic::ImguiText
            | Intrinsic::ImguiTextColored
            | Intrinsic::ImguiButton
            | Intrinsic::ImguiCheckbox
            | Intrinsic::ImguiRadio
            | Intrinsic::ImguiSlider
            | Intrinsic::ImguiDrag
            | Intrinsic::ImguiInputText
            | Intrinsic::ImguiSeparator
            | Intrinsic::ImguiSameLine
            | Intrinsic::ImguiTree
            | Intrinsic::ImguiTreeEnd
            | Intrinsic::ImguiCollapsingHeader => self.imgui(intrinsic, name, args),

            // Both play out over time, and a bake shows the appearance from before either starts
            Intrinsic::Animate | Intrinsic::Flick => Ok(GenericValue::Null),

            Intrinsic::Browse
            | Intrinsic::BrowseRsc
            | Intrinsic::Fcopy
            | Intrinsic::Fdel
            | Intrinsic::Ftp
            | Intrinsic::Link
            | Intrinsic::Shell
            | Intrinsic::Shutdown
            | Intrinsic::Sleep
            | Intrinsic::Text2file
            | Intrinsic::Winget
            | Intrinsic::Winset => Err(self.fault(FaultKind::Blocked(name.into()))),

            Intrinsic::Image | Intrinsic::MutableAppearance => self.appearance_object(intrinsic, None, params, args),
            Intrinsic::ImageNew => self.appearance_object(intrinsic, receiver.object(), params, args),
            Intrinsic::Issaved => Ok(true.into()),

            Intrinsic::FcopyRsc => Ok(arg(0)),

            Intrinsic::ListAdd
            | Intrinsic::ListCopy
            | Intrinsic::ListCut
            | Intrinsic::ListFind
            | Intrinsic::ListInsert
            | Intrinsic::ListJoin
            | Intrinsic::ListRemove
            | Intrinsic::ListRemoveAll
            | Intrinsic::ListSwap
            | Intrinsic::ListSplice => {
                let id = receiver.list().ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                self.list_builtin(intrinsic, id, args.into_iter().map(|(_, value)| value).collect())
            },

            Intrinsic::Nameof => match arg(0) {
                GenericValue::Path(path) => match path.name() {
                    Some(name) => self.text(name.to_string()),
                    None => Ok(GenericValue::Null),
                },
                _ => Err(self.fault(FaultKind::Blocked("nameof of a non-path".into()))),
            },

            Intrinsic::File2text => match self.resource_path(&arg(0))? {
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(text) if text.len() > self.limits.text_bytes => Err(self.fault(FaultKind::Memory)),
                    Ok(text) => self.text(text),
                    Err(_) => Ok(GenericValue::Null),
                },
                None => Ok(GenericValue::Null),
            },
            Intrinsic::Fexists => Ok(self.resource_path(&arg(0))?.is_some_and(|path| path.is_file()).into()),
            Intrinsic::File => match self.resource_path(&arg(0))? {
                Some(_) => Ok(GenericValue::Resource(text(0).into())),
                None => Ok(GenericValue::Null),
            },
            Intrinsic::Flist => {
                let Some(path) = self.resource_path(&arg(0))? else {
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
            },

            Intrinsic::Isnull => Ok(matches!(arg(0), GenericValue::Null).into()),
            Intrinsic::Isnum => Ok(matches!(arg(0), GenericValue::Num(_)).into()),
            Intrinsic::Istext => Ok(matches!(arg(0), GenericValue::Text(_)).into()),
            Intrinsic::Islist => Ok(matches!(arg(0), GenericValue::List(_) | GenericValue::ArgList(_)).into()),
            Intrinsic::Isicon => Ok(match arg(0) {
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
            Intrinsic::Ispath => {
                let ty = match arg(0) {
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
            Intrinsic::Istype => {
                let value = arg(0);
                let base_path = match arg(1) {
                    GenericValue::Path(path) => Some(path),
                    _ => None,
                };
                let matches = match &value {
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
                };
                Ok(matches.into())
            },
            Intrinsic::Isturf
            | Intrinsic::Isarea
            | Intrinsic::Ismovable
            | Intrinsic::Isobj
            | Intrinsic::Ismob
            | Intrinsic::Isloc => {
                let roots = self.tree.roots();
                let root = match intrinsic {
                    Intrinsic::Isturf => roots.turf,
                    Intrinsic::Isarea => roots.area,
                    Intrinsic::Ismovable => roots.movable,
                    Intrinsic::Isobj => roots.obj,
                    Intrinsic::Ismob => roots.mob,
                    _ => roots.atom,
                };

                Ok(self.is_root_type(&arg(0), root).into())
            },
            Intrinsic::Length | Intrinsic::LengthChar => Ok((match arg(0) {
                GenericValue::List(id) | GenericValue::ArgList(id) => {
                    self.runtime.heap.list(id).map_or(0, |l| l.entries.len())
                },
                GenericValue::Text(s) | GenericValue::Resource(s) => s.chars().count(),
                GenericValue::Null => 0,
                GenericValue::Object(id) => self.runtime.heap.object(id).map_or(0, |o| o.contents.len()),
                _ => 0,
            } as f32)
                .into()),
            Intrinsic::Typesof => self.types_of(positional(), true),
            Intrinsic::Text2path => {
                let path = TreePath::parse(text(0));
                Ok(self
                    .tree
                    .id_of(&path)
                    .map(|_| GenericValue::Path(path))
                    .unwrap_or_default())
            },
            Intrinsic::Min | Intrinsic::Max => {
                let args = positional();
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
            },
            Intrinsic::Abs => Ok(self.number(&arg(0))?.abs().into()),
            Intrinsic::Ceil => Ok(self.number(&arg(0))?.ceil().into()),
            Intrinsic::Floor => Ok(self.number(&arg(0))?.floor().into()),
            Intrinsic::Sqrt => Ok(self.number(&arg(0))?.sqrt().into()),
            Intrinsic::Round => {
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
            },
            Intrinsic::Clamp => {
                let n = self.number(&arg(0))?;
                let lo = self.number(&arg(1))?;
                let hi = self.number(&arg(2))?;
                if lo.is_nan() || hi.is_nan() || lo > hi {
                    return Err(self.fault(FaultKind::InvalidOperation("invalid clamp bounds".into())));
                }

                Ok(n.clamp(lo, hi).into())
            },
            Intrinsic::Rand => {
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
            },
            Intrinsic::Prob => {
                let n = self.number(&arg(0))?;

                Ok((self.random() * 100.0 < n).into())
            },
            Intrinsic::GetStep => {
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
                    self.memo_safe = false;
                }

                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.world.get_step(&self.runtime.heap, id, dir))
                    .map(GenericValue::Object)
                    .unwrap_or_default())
            },
            Intrinsic::GetDir | Intrinsic::GetDist => {
                self.position_sensitive = true;
                let pos = |value: GenericValue, evaluator: &Self| {
                    value
                        .object()
                        .and_then(|id| evaluator.runtime.world.position(&evaluator.runtime.heap, id))
                };
                let (Some(a), Some(b)) = (pos(arg(0), self), pos(arg(1), self)) else {
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
            },
            Intrinsic::Locate => {
                if args.len() >= 3 || args.len() < 2 {
                    self.memo_safe = false;
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

                let values = self.iter_values(args.get(1).map(|(_, v)| v.clone()).unwrap_or(GenericValue::World))?;
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
            Intrinsic::Turn => {
                let dirs = [1, 9, 8, 10, 2, 6, 4, 5];
                let dir = self.number(&arg(0))? as i32;
                let angle = self.number(&arg(1))?;
                let Some(index) = dirs.iter().position(|d| *d == dir) else {
                    return Ok(dir.into());
                };
                let index = (index as i32 + (angle.rem_euclid(360.0) / 45.0).round() as i32).rem_euclid(8) as usize;

                Ok(dirs.get(index).copied().unwrap_or(0).into())
            },
            Intrinsic::Rgb
                if (3..=4).contains(&args.len()) && args.iter().all(|(_, a)| matches!(a, GenericValue::Num(_))) =>
            {
                let mut color = String::from("#");
                for index in 0..args.len() {
                    let channel = self.number(&arg(index))?.round().clamp(0.0, 255.0) as u8;
                    color.push_str(&format!("{channel:02x}"));
                }

                self.text(color)
            },
            Intrinsic::Rgb => Err(self.fault(FaultKind::Blocked(name.into()))),
            Intrinsic::Num2text => {
                let n = self.number(&arg(0))?;

                self.text(n.to_string())
            },
            Intrinsic::Text2num => Ok(text(0).trim().parse::<f32>().map(GenericValue::Num).unwrap_or_default()),
            Intrinsic::Uppertext => {
                let t = text(0).to_uppercase();

                self.text(t)
            },
            Intrinsic::Lowertext => {
                let t = text(0).to_lowercase();

                self.text(t)
            },
            Intrinsic::Sorttext | Intrinsic::SorttextEx => {
                self.charge(text(0).len().saturating_add(text(1).len()))?;
                let order = if intrinsic == Intrinsic::SorttextEx {
                    text(1).cmp(text(0))
                } else {
                    text(1).to_lowercase().cmp(&text(0).to_lowercase())
                };

                Ok((order as i32).into())
            },
            Intrinsic::Copytext | Intrinsic::CopytextChar => {
                let s = text(0);
                let chars = s.chars().collect::<Vec<_>>();
                self.charge(chars.len())?;
                let args = positional();
                let start = index_arg(&args, 1, 1, chars.len() + 1);
                let end = index_arg(&args, 2, 0, chars.len() + 1);

                self.text(chars.get(start.min(end)..end).unwrap_or_default().iter().collect())
            },
            Intrinsic::Findtext | Intrinsic::FindtextEx | Intrinsic::Findlasttext | Intrinsic::FindlasttextEx => {
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
                let args = positional();
                let start = index_arg(&args, 2, 1, chars.len() + 1);
                let end = index_arg(&args, 3, 0, chars.len() + 1);
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
            },
            Intrinsic::Replacetext | Intrinsic::ReplacetextEx => {
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
            },
            Intrinsic::Splittext => {
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
            Intrinsic::JsonDecode => {
                let value = crate::json::decode(
                    text(0),
                    self.limits.allocations.min(self.instruction_budget_remaining as usize),
                )
                .map_err(|e| self.fault(e))?;
                self.reserve(text(0).len())?;

                self.constant(&value)
            },
            Intrinsic::Ref => {
                self.memo_safe = false;

                self.text(arg(0).display())
            },
            Intrinsic::Hascall => {
                let name = Identifier::from(text(1));

                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.heap.object(id))
                    .and_then(|o| self.find_proc(o.ty, &name))
                    .is_some()
                    .into())
            },
            Intrinsic::Crash => Err(self.fault(FaultKind::InvalidOperation(arg(0).display()))),
            Intrinsic::Icon => self.construct("/icon", params, None, args),
            Intrinsic::Sound => self.construct("/sound", params, None, args),
            Intrinsic::Matrix => {
                let matrix = self.tree.id_of(&TreePath::parse("/matrix"));
                match self.is_root_type(&arg(0), matrix) {
                    true => Ok(arg(0)),
                    false => self.construct("/matrix", params, None, args),
                }
            },
            Intrinsic::Regex => {
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
                            .alloc_object(crate::heap::Object::new(ty))
                            .map_err(|kind| self.fault(kind))?
                    },
                };

                self.write_field(GenericValue::Object(id), "name".into(), pattern)?;
                self.write_field(GenericValue::Object(id), "flags".into(), flags)?;

                Ok(GenericValue::Object(id))
            },

            Intrinsic::Trimtext => {
                let trimmed = text(0).trim().to_string();

                self.text(trimmed)
            },
            Intrinsic::Ckey | Intrinsic::CkeyEx => {
                let fold = intrinsic == Intrinsic::Ckey;
                let key = text(0)
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .map(|c| if fold { c.to_ascii_lowercase() } else { c })
                    .collect::<String>();

                self.text(key)
            },
            Intrinsic::Cmptext | Intrinsic::CmptextEx => {
                let exact = intrinsic == Intrinsic::CmptextEx;
                let first = text(0);
                self.charge(args.len())?;

                Ok((1..args.len())
                    .all(|index| match exact {
                        true => text(index) == first,
                        false => text(index).eq_ignore_ascii_case(first),
                    })
                    .into())
            },
            Intrinsic::Addtext => {
                let mut output = String::new();
                for (_, value) in &args {
                    let piece = value.display();
                    if output.len().saturating_add(piece.len()) > self.limits.text_bytes {
                        return Err(self.fault(FaultKind::Memory));
                    }

                    output.push_str(&piece);
                }

                self.text(output)
            },
            Intrinsic::Jointext => {
                let entries = self.iter_values(arg(0))?;
                let glue = text(1).to_string();
                let positional = positional();
                let start = index_arg(&positional, 2, 1, entries.len() + 1);
                let end = index_arg(&positional, 3, 0, entries.len() + 1);

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
            },
            Intrinsic::Text2ascii | Intrinsic::Text2asciiChar => {
                let position = match args.len() {
                    0 | 1 => 1.0,
                    _ => self.number(&arg(1))?,
                };
                let chars = text(0).chars().collect::<Vec<_>>();
                let index = index_arg(&[GenericValue::Num(position)], 0, 1, chars.len() + 1);

                Ok(chars.get(index).map(|c| *c as u32 as f32).unwrap_or(0.0).into())
            },
            Intrinsic::Ascii2text => {
                let code = self.number(&arg(0))?;
                let Some(character) = u32::try_from(code as i64).ok().and_then(char::from_u32) else {
                    return Ok(GenericValue::Null);
                };

                self.text(character.to_string())
            },

            Intrinsic::Sin => Ok(self.number(&arg(0))?.to_radians().sin().into()),
            Intrinsic::Cos => Ok(self.number(&arg(0))?.to_radians().cos().into()),
            Intrinsic::Tan => Ok(self.number(&arg(0))?.to_radians().tan().into()),
            Intrinsic::Arcsin => Ok(self.number(&arg(0))?.asin().to_degrees().into()),
            Intrinsic::Arccos => Ok(self.number(&arg(0))?.acos().to_degrees().into()),
            Intrinsic::Arctan => {
                let a = self.number(&arg(0))?;

                Ok(match args.len() {
                    0 | 1 => a.atan().to_degrees(),
                    _ => self.number(&arg(1))?.atan2(a).to_degrees(),
                }
                .into())
            },
            Intrinsic::Log => {
                let first = self.number(&arg(0))?;

                Ok(match args.len() {
                    0 | 1 => first.ln(),
                    _ => self.number(&arg(1))?.log(first),
                }
                .into())
            },
            Intrinsic::Sign => Ok(match self.number(&arg(0))? {
                n if n > 0.0 => 1.0,
                n if n < 0.0 => -1.0,
                _ => 0.0,
            }
            .into()),
            Intrinsic::Lerp => {
                let from = self.number(&arg(0))?;
                let to = self.number(&arg(1))?;
                let factor = self.number(&arg(2))?;

                Ok((from + (to - from) * factor).into())
            },
            Intrinsic::Fract => Ok(self.number(&arg(0))?.fract().into()),
            Intrinsic::Trunc => Ok(self.number(&arg(0))?.trunc().into()),
            Intrinsic::Isnan => Ok(self.number(&arg(0))?.is_nan().into()),
            Intrinsic::Isinf => Ok(self.number(&arg(0))?.is_infinite().into()),

            Intrinsic::Alist => {
                let entries = args
                    .iter()
                    .map(|(key, value)| match key {
                        Some(key) => (GenericValue::Text(key.as_str().into()), Some(value.clone())),
                        None => (value.clone(), None),
                    })
                    .collect::<Vec<_>>();

                self.alist(entries)
            },

            Intrinsic::Alert
            | Intrinsic::Astype
            | Intrinsic::Block
            | Intrinsic::BoundPixloc
            | Intrinsic::BoundsDist
            | Intrinsic::Bounds
            | Intrinsic::DmDbClose
            | Intrinsic::DmDbColumns
            | Intrinsic::DmDbConnect
            | Intrinsic::DmDbErrorMsg
            | Intrinsic::DmDbExecute
            | Intrinsic::DmDbIsConnected
            | Intrinsic::DmDbNewCon
            | Intrinsic::DmDbNewQuery
            | Intrinsic::DmDbNextRow
            | Intrinsic::DmDbQuote
            | Intrinsic::DmDbRowCount
            | Intrinsic::DmDbRowsAffected
            | Intrinsic::Filter
            | Intrinsic::FindlasttextChar
            | Intrinsic::FindlasttextExChar
            | Intrinsic::FindtextChar
            | Intrinsic::FindtextExChar
            | Intrinsic::Ftime
            | Intrinsic::GetStepAway
            | Intrinsic::GetStepRand
            | Intrinsic::GetStepsTo
            | Intrinsic::GetStepTo
            | Intrinsic::GetStepTowards
            | Intrinsic::Generator
            | Intrinsic::Gradient
            | Intrinsic::Hearers
            | Intrinsic::HtmlDecode
            | Intrinsic::HtmlEncode
            | Intrinsic::Isfile
            | Intrinsic::Ispointer
            | Intrinsic::JsonEncode
            | Intrinsic::Lentext
            | Intrinsic::List2params
            | Intrinsic::LoadExt
            | Intrinsic::LoadResource
            | Intrinsic::Md5
            | Intrinsic::Missile
            | Intrinsic::Newlist
            | Intrinsic::NoiseHash
            | Intrinsic::NonspantextChar
            | Intrinsic::Nonspantext
            | Intrinsic::Obounds
            | Intrinsic::Ohearers
            | Intrinsic::Output
            | Intrinsic::Oview
            | Intrinsic::Oviewers
            | Intrinsic::Params2list
            | Intrinsic::Pixloc
            | Intrinsic::RandSeed
            | Intrinsic::Refcount
            | Intrinsic::ReplacetextChar
            | Intrinsic::ReplacetextExChar
            | Intrinsic::Rgb2num
            | Intrinsic::Roll
            | Intrinsic::Run
            | Intrinsic::Sha1
            | Intrinsic::SpantextChar
            | Intrinsic::Spantext
            | Intrinsic::SplicetextChar
            | Intrinsic::Splicetext
            | Intrinsic::SplittextChar
            | Intrinsic::Startup
            | Intrinsic::Stat
            | Intrinsic::Statpanel
            | Intrinsic::StepAway
            | Intrinsic::StepRand
            | Intrinsic::Step
            | Intrinsic::StepTo
            | Intrinsic::StepTowards
            | Intrinsic::Text
            | Intrinsic::Time2text
            | Intrinsic::UrlDecode
            | Intrinsic::UrlEncode
            | Intrinsic::ValuesCutOver
            | Intrinsic::ValuesCutUnder
            | Intrinsic::ValuesDot
            | Intrinsic::ValuesProduct
            | Intrinsic::ValuesSum
            | Intrinsic::Vector
            | Intrinsic::View
            | Intrinsic::Viewers
            | Intrinsic::WalkAway
            | Intrinsic::WalkRand
            | Intrinsic::Walk
            | Intrinsic::WalkTo
            | Intrinsic::WalkTowards
            | Intrinsic::Winclone
            | Intrinsic::Winexists
            | Intrinsic::Winshow => Err(self.fault(FaultKind::Blocked(name.into()))),
        }
    }

    fn construct(
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
                    .alloc_object(crate::heap::Object::new(ty))
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
    fn range_list(&mut self, args: [GenericValue; 2], include_center: bool) -> Result<GenericValue> {
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
            self.memo_safe = false;
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
            self.memo_safe = false;
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
            for (x, y) in tiles {
                if let Some(turf) = self.runtime.world.turf_at(Position::new(x, y, origin.z)) {
                    self.range_add_with_contents(&mut entries, &mut areas, turf, None)?;
                }
            }
        }

        self.range_finish(entries)
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

    fn range_add_with_contents(
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
    fn range_add(
        &mut self, entries: &mut Vec<GenericValue>, areas: &mut HashSet<ObjectId>, id: ObjectId,
    ) -> Result<()> {
        let invisibility = self.read_field(GenericValue::Object(id), &Identifier::from("invisibility"))?;
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

    fn resource_path(&mut self, file: &GenericValue) -> Result<Option<PathBuf>> {
        let Some(name) = file.text() else {
            return Ok(None);
        };
        let Some(root) = self.runtime.world.root.clone() else {
            return Err(self.fault(FaultKind::Blocked("filesystem".into())));
        };

        let path = root.join(name);
        match path.starts_with(&root) {
            true => Ok(Some(path)),
            false => Err(self.fault(FaultKind::Blocked("filesystem".into()))),
        }
    }

    fn file2list(&mut self, file: &GenericValue, separator: &GenericValue) -> Result<GenericValue> {
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
}

/// OpenDream's `DantomSpiral`: rings outward from the centre, each one left column, then the rows
/// below and above, then right column.
fn dantom_spiral(center: (i32, i32), size: (i32, i32)) -> Vec<(i32, i32)> {
    let (width, height) = size;
    let bottom = height / 2;
    let top = (height - 1) / 2;
    let left = width / 2;
    let right = (width - 1) / 2;
    let mut tiles = Vec::new();

    for ring in 1..=bottom.max(left) {
        let column_bottom = center.1 - ring.min(bottom);
        let column_top = center.1 + ring.min(top);
        if ring <= left {
            tiles.extend((column_bottom..=column_top).map(|y| (center.0 - ring, y)));
        }

        if ring <= bottom {
            let first = center.0 - ring.min(left + 1) + 1;
            let last = center.0 + ring.min(right + 1) - 1;
            for x in first..=last {
                tiles.push((x, center.1 - ring));
                if ring <= top {
                    tiles.push((x, center.1 + ring));
                }
            }
        }

        if ring <= right {
            tiles.extend((column_bottom..=column_top).map(|y| (center.0 + ring, y)));
        }
    }

    tiles
}
