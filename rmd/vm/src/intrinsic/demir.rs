use core::types::Identifier;
use std::collections::hash_map::Entry;

use defines::{EAST, NORTH, NORTHEAST, NORTHWEST, SOUTH, SOUTHEAST, SOUTHWEST, WEST};

use super::Result;
use crate::{
    FaultKind, GenericValue,
    bake::{NodeGroup, NodeOrientation, PlacementState, PlacementTarget, Rotation, placement_entry},
    eval::Evaluator,
    toml_value,
};

impl Evaluator<'_> {
    pub(crate) fn profile(&mut self) -> Result<GenericValue> {
        Ok(self.runtime.profile.map(GenericValue::Object).unwrap_or_default())
    }

    pub(crate) fn node_group(
        &mut self, name: &str, subtype_arg: GenericValue, blocker_arg: GenericValue, orientable_arg: GenericValue,
        matching_arg: GenericValue,
    ) -> Result<GenericValue> {
        if !self.runtime.defining_groups {
            return Err(self.fault(FaultKind::Blocked(format!("{name} outside a profile's New()"))));
        }

        let invalid = |what: &str, value: &GenericValue| {
            FaultKind::InvalidOperation(format!("{name} {what} must be a type, got {}", value.display()))
        };

        // which object
        let Some(subtype) = (match &subtype_arg {
            GenericValue::Path(path) => self.tree.id_of(path),
            GenericValue::Object(id) => self.runtime.heap.object(*id).map(|object| object.ty),
            _ => None,
        }) else {
            return Err(self.fault(invalid("subtype", &subtype_arg)));
        };

        // which blocker
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
        let mut blockers = Vec::with_capacity(blocker_values.len());
        for value in &blocker_values {
            let Some(blocker) = resolve_type(value) else {
                return Err(self.fault(invalid("blocker", value)));
            };

            blockers.push(blocker);
        }

        // which orentation (optional)
        let orientable_subtype = match orientable_arg {
            GenericValue::Null => None,
            value => {
                let Some(ty) = resolve_type(&value).filter(|ty| self.tree.is_subtype_of(*ty, subtype)) else {
                    return Err(self.fault(FaultKind::InvalidOperation(format!(
                        "{name} orientable_subtype must be a descendant of the subtype, got {}",
                        value.display()
                    ))));
                };

                Some(ty)
            },
        };

        // which objects with same values/fields
        let matching_values = match matching_arg {
            GenericValue::Null => Vec::new(),
            value @ (GenericValue::List(_) | GenericValue::ArgList(_)) => {
                self.iter_values(value)?.into_iter().map(|(value, _)| value).collect()
            },
            value => vec![value],
        };
        let mut matching = Vec::with_capacity(matching_values.len());
        for value in &matching_values {
            let Some(text) = value.text() else {
                return Err(self.fault(FaultKind::InvalidOperation(format!(
                    "{name} matching must name vars as text, got {}",
                    value.display()
                ))));
            };

            matching.push(Identifier::from(text));
        }

        if let Some(group) = self
            .runtime
            .node_groups
            .iter_mut()
            .find(|group| group.subtype == subtype)
        {
            if orientable_subtype.is_some()
                && group.orientable_subtype.is_some()
                && group.orientable_subtype != orientable_subtype
            {
                return Ok(GenericValue::Null);
            }

            group.orientable_subtype = group.orientable_subtype.or(orientable_subtype);
            for blocker in blockers {
                if !group.blockers.contains(&blocker) {
                    group.blockers.push(blocker);
                }
            }

            for name in matching {
                if !group.matching.contains(&name) {
                    group.matching.push(name);
                }
            }
        } else {
            // new group
            self.runtime.node_groups.push(NodeGroup {
                subtype,
                orientable_subtype,
                orientations: Vec::new(),
                blockers: blockers.into_iter().fold(Vec::new(), |mut unique, blocker| {
                    if !unique.contains(&blocker) {
                        unique.push(blocker);
                    }

                    unique
                }),
                matching: matching.into_iter().fold(Vec::new(), |mut unique, name| {
                    if !unique.contains(&name) {
                        unique.push(name);
                    }

                    unique
                }),
            });
        }

        Ok(GenericValue::Null)
    }

    pub(crate) fn node_orientation(
        &mut self, name: &str, args: &[(Option<Identifier>, GenericValue)],
    ) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).map(|(_, value)| value.clone()).unwrap_or_default();

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

        let number = |value: GenericValue| match value {
            GenericValue::Num(number)
                if number.is_finite() && number >= 0.0 && number.fract() == 0.0 && number < u32::MAX as f32 =>
            {
                Some(number as u32)
            },
            _ => None,
        };

        let (Some(direction), Some(openings)) = (number(arg(1)), number(arg(2))) else {
            return Ok(GenericValue::Null);
        };

        if openings & !0xf != 0 {
            return Ok(GenericValue::Null);
        }

        let Some(index) = self
            .runtime
            .node_groups
            .iter()
            .enumerate()
            .filter(|(_, group)| self.tree.is_subtype_of(subtype, group.subtype))
            .max_by_key(|(_, group)| self.tree.ancestors(group.subtype).count())
            .map(|(index, _)| index)
        else {
            return Ok(GenericValue::Null);
        };
        let rules = &mut self.runtime.node_groups[index].orientations;
        if let Some(rule) = rules
            .iter_mut()
            .find(|rule| rule.subtype == subtype && rule.direction == direction)
        {
            rule.openings = openings;
        } else {
            rules.push(NodeOrientation {
                subtype,
                direction,
                openings,
            });
        }
        Ok(GenericValue::Null)
    }

    pub(crate) fn rotatable(
        &mut self, name: &str, args: &[(Option<Identifier>, GenericValue)],
    ) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).map(|(_, value)| value.clone()).unwrap_or_default();

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

        let values = match arg(1) {
            value @ (GenericValue::List(_) | GenericValue::ArgList(_)) => {
                self.iter_values(value)?.into_iter().map(|(value, _)| value).collect()
            },
            value => vec![value],
        };
        let mut directions = Vec::new();
        for value in values {
            let Some(direction) = value
                .num()
                .filter(|number| number.fract() == 0.0 && *number >= 0.0)
                .map(|number| number as u32)
            else {
                continue;
            };
            if matches!(
                direction,
                NORTH | SOUTH | EAST | WEST | NORTHEAST | NORTHWEST | SOUTHEAST | SOUTHWEST
            ) && !directions.contains(&direction)
            {
                directions.push(direction);
            }
        }
        if directions.is_empty() {
            return Ok(GenericValue::Null);
        }

        if let Some(rotation) = self
            .runtime
            .rotations
            .iter_mut()
            .find(|rotation| rotation.subtype == subtype)
        {
            rotation.directions = directions;
        } else {
            self.runtime.rotations.push(Rotation { subtype, directions });
        }

        Ok(GenericValue::Null)
    }

    pub(crate) fn modular_root(
        &mut self, name: &str, args: &[(Option<Identifier>, GenericValue)],
    ) -> Result<GenericValue> {
        if !self.runtime.defining_groups {
            return Err(self.fault(FaultKind::Blocked(format!("{name} outside a profile's New()"))));
        }

        let ty = match args.first().map(|(_, value)| value.clone()) {
            Some(GenericValue::Path(path)) => self.tree.id_of(&path),
            Some(GenericValue::Object(id)) => self.runtime.heap.object(id).map(|object| object.ty),
            _ => None,
        };

        if let Some(ty) = ty
            && !self.runtime.modular_roots.contains(&ty)
        {
            self.runtime.modular_roots.push(ty);
        }

        Ok(GenericValue::Null)
    }

    pub(crate) fn place_modular(&mut self, list: GenericValue) -> Result<GenericValue> {
        for (value, _) in self.iter_values(list)? {
            let (target, placement) = placement_entry(self, value)?;
            let root = match target {
                PlacementTarget::Root(root) => root,
                PlacementTarget::At(position) => {
                    if !self.runtime.defining_groups {
                        return Err(self.fault(FaultKind::Blocked(
                            "demir_place_modular() at outside a profile's New()".into(),
                        )));
                    }

                    match self.runtime.coordinate_modules.entry(position) {
                        Entry::Occupied(_) => {
                            return Err(self.fault(FaultKind::InvalidOperation(
                                "demir_place_modular() at already has an entry".into(),
                            )));
                        },
                        Entry::Vacant(entry) => {
                            entry.insert(placement);
                        },
                    }

                    continue;
                },
            };
            let message = match &mut self.runtime.placement {
                PlacementState::Task { roots, entries } => {
                    if !roots.contains(&root) {
                        Some("demir_place_modular() root is not an atom of this pass")
                    } else {
                        match entries.entry(root) {
                            Entry::Occupied(_) => Some("demir_place_modular() root already has an entry in this pass"),
                            Entry::Vacant(entry) => {
                                entry.insert(placement);

                                None
                            },
                        }
                    }
                },
                PlacementState::Idle => {
                    let is_placed = self
                        .runtime
                        .heap
                        .object(root)
                        .is_some_and(|object| object.instance.is_some());
                    if is_placed {
                        self.runtime.pending_placements.push(root, placement);

                        None
                    } else {
                        Some("demir_place_modular() root must be an atom placed on the map")
                    }
                },
            };
            if let Some(message) = message {
                return Err(self.fault(FaultKind::InvalidOperation(message.into())));
            }
        }

        Ok(GenericValue::Null)
    }

    pub(crate) fn toml_decode(&mut self, text: &str) -> Result<GenericValue> {
        let value = match toml_value::decode(
            text,
            self.limits.allocations.min(self.instruction_budget_remaining as usize),
        ) {
            Ok(value) => value,
            Err(FaultKind::InvalidOperation(message)) => return Err(self.throw_exception(message)),
            Err(kind) => return Err(self.fault(kind)),
        };
        self.reserve(text.len())?;

        self.constant(&value)
    }

    pub(crate) fn define_group(
        &mut self, name: &str, args: &[(Option<Identifier>, GenericValue)],
    ) -> Result<GenericValue> {
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
    }

    pub(crate) fn map_path(&mut self) -> Result<GenericValue> {
        match self.runtime.world.map_path.clone() {
            Some(path) => self.text(path),
            None => Ok(GenericValue::Null),
        }
    }
}
