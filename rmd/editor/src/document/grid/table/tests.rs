use base::{path::TreePath, types::Value};

use dmm::Prefab;

use super::PrefabTable;

fn sign(vars: &[(&str, Value)]) -> Prefab {
    let mut prefab = Prefab::new(TreePath::parse("/obj/sign"));
    for (name, value) in vars {
        prefab.set_var((*name).into(), value.clone());
    }

    prefab
}

#[test]
fn equal_prefabs_intern_to_one_entry() {
    let mut table = PrefabTable::default();
    let east = sign(&[("dir", Value::Num(4.0))]);

    let first = table.intern(&east);
    let second = table.intern(&east.clone());
    let plain = table.intern(&sign(&[]));

    assert_eq!(first, second);
    assert_ne!(first, plain);
    assert_eq!(table.get(first), &east);
    assert_eq!(table.find(&east), Some(first));
    assert_eq!(table.find(&sign(&[("dir", Value::Num(8.0))])), None);
}

#[test]
fn negative_zero_is_zero_but_var_order_matters() {
    let mut table = PrefabTable::default();
    let zero = table.intern(&sign(&[("pixel_x", Value::Num(0.0))]));
    let ordered = table.intern(&sign(&[("dir", Value::Num(4.0)), ("name", Value::Text("a".into()))]));

    assert_eq!(table.intern(&sign(&[("pixel_x", Value::Num(-0.0))])), zero);
    assert_ne!(
        table.intern(&sign(&[("name", Value::Text("a".into())), ("dir", Value::Num(4.0))])),
        ordered
    );
}

#[test]
fn nan_never_equals_itself_so_each_intern_adds_an_entry() {
    let mut table = PrefabTable::default();
    let nan = sign(&[("pixel_x", Value::Num(f32::NAN))]);

    assert_ne!(table.intern(&nan), table.intern(&nan));
}

#[test]
fn live_prefabs_are_the_placed_and_pinned_ones() {
    let mut table = PrefabTable::default();
    let placed = table.intern(&sign(&[]));
    let pinned = table.intern(&sign(&[("dir", Value::Num(4.0))]));
    let removed = table.intern(&sign(&[("dir", Value::Num(8.0))]));

    table.place(placed);
    table.place(placed);
    table.place(removed);
    table.unplace(removed);
    table.pin(pinned);

    assert_eq!(table.placed(placed), 2);
    assert_eq!(table.placed(removed), 0);
    assert_eq!(table.live().collect::<Vec<_>>(), [table.get(placed), table.get(pinned)]);
}

#[test]
#[should_panic(expected = "unplaced a prefab that was never placed")]
fn unplacing_below_zero_panics() {
    let mut table = PrefabTable::default();
    let id = table.intern(&sign(&[]));

    table.unplace(id);
}
