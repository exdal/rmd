use base::{
    bitset::BitIndex,
    coord::Coord,
    path::TreePath,
    types::{Identifier, Value},
};
use std::{
    hash::{Hash, Hasher},
    num::NonZeroU64,
};

pub mod table;

/// `charge = 2e+005`, `damage = 1.30`, `network = list("ss13", "engine")`
#[derive(Debug, Clone, PartialEq)]
pub struct VarValue {
    pub value: Value,
    spelling: Option<String>,
}

impl VarValue {
    pub fn new(value: Value) -> Self {
        Self { value, spelling: None }
    }

    pub fn spelling(&self) -> Option<&str> {
        self.spelling.as_deref()
    }
}

impl Hash for VarValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl From<Value> for VarValue {
    fn from(value: Value) -> Self {
        Self::new(value)
    }
}

/// Vars stay in file order
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct Prefab {
    pub path: TreePath,
    pub vars: Vec<(Identifier, VarValue)>,
}

impl Prefab {
    pub fn new(path: TreePath) -> Self {
        Self { path, vars: Vec::new() }
    }

    pub fn var(&self, name: &Identifier) -> Option<&Value> {
        self.vars.iter().find(|(key, _)| key == name).map(|(_, var)| &var.value)
    }

    pub fn set_var(&mut self, name: Identifier, value: Value) {
        self.set(name, VarValue::new(value));
    }

    pub fn remove_var(&mut self, name: &Identifier) -> Option<VarValue> {
        let index = self.vars.iter().position(|(key, _)| key == name)?;

        Some(self.vars.remove(index).1)
    }

    pub fn set_var_with_spelling(&mut self, name: Identifier, value: Value, spelling: Option<String>) {
        self.set(name, VarValue { value, spelling });
    }

    fn set(&mut self, name: Identifier, var: VarValue) {
        match self.vars.iter_mut().find(|(key, _)| *key == name) {
            Some(slot) => slot.1 = var,
            None => self.vars.push((name, var)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrefabId(u32);

/// Stable runtime identity for one prefab placement in a map document.
///
/// Placement IDs are editor state and are not serialized into DMM files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrefabInstanceId(NonZeroU64);

impl PrefabInstanceId {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl BitIndex for PrefabInstanceId {
    fn bit_index(self) -> usize {
        self.get() as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefabLocation {
    pub coord: Coord,
    pub prefab_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrefabInstance<'a> {
    pub id: PrefabInstanceId,
    pub location: PrefabLocation,
    prefab: &'a Prefab,
}

impl<'a> PrefabInstance<'a> {
    pub fn new(id: PrefabInstanceId, prefab: &'a Prefab, location: PrefabLocation) -> Self {
        Self { id, location, prefab }
    }

    pub fn prefab(&self) -> &'a Prefab {
        self.prefab
    }
}

pub fn prefabs_equal(left: &Prefab, right: &Prefab) -> bool {
    left.path == right.path
        && left.vars.len() == right.vars.len()
        && left
            .vars
            .iter()
            .zip(&right.vars)
            .all(|((left_name, left), (right_name, right))| left_name == right_name && left.value == right.value)
}

#[cfg(test)]
mod tests {
    use base::{path::TreePath, types::Value};

    use crate::Prefab;

    #[test]
    fn prefab_vars_keep_insertion_order_and_replace_in_place() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/t"));
        prefab.set_var("name".into(), Value::Num(1.0));
        prefab.set_var("id_tag".into(), Value::Num(2.0));
        prefab.set_var("name".into(), Value::Num(3.0));

        let names: Vec<&str> = prefab.vars.iter().map(|(name, _)| name.as_str()).collect();

        assert_eq!(names, ["name", "id_tag"]);
        assert_eq!(prefab.var(&"name".into()), Some(&Value::Num(3.0)));
    }

    #[test]
    fn removing_a_prefab_var_preserves_the_order_of_the_rest() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/t"));
        prefab.set_var("first".into(), Value::Num(1.0));
        prefab.set_var("middle".into(), Value::Num(2.0));
        prefab.set_var("last".into(), Value::Num(3.0));

        assert_eq!(
            prefab.remove_var(&"middle".into()).map(|var| var.value),
            Some(Value::Num(2.0)),
        );
        assert_eq!(
            prefab.vars.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
            ["first", "last"],
        );
        assert_eq!(prefab.remove_var(&"missing".into()), None);
    }
}
