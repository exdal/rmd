use core::{arena::StrArena, path::TreePath};
use std::sync::atomic::{AtomicUsize, Ordering};

use objtree::{ObjectTree, TypeId};

use crate::{
    FaultKind,
    GenericValue,
    Limits,
    Runtime,
    eval::Evaluator,
    heap::{Object, ObjectId},
};

fn compile(source: &str) -> (ObjectTree, codegen::Module) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rmd-vm-fixture-{}-{id}.dm", std::process::id()));
    std::fs::write(&path, source).expect("write fixture");
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, &path).expect("fixture should preprocess");
    let _ = std::fs::remove_file(path);
    assert!(preprocessed.errors.is_empty(), "{:?}", preprocessed.errors);
    let ast = ast::parse(&preprocessed.tokens).expect("fixture should parse");
    let (tree, module, errors) = sema::analyze(&ast);
    assert!(errors.is_empty(), "{errors:?}");
    let module = codegen::generate(&module).expect("fixture should compile");
    (tree, module)
}

fn proc(tree: &ObjectTree, name: &str) -> core::types::ProcId {
    tree.proc_inherited(TypeId::ROOT, &name.into())
        .and_then(|proc| proc.body)
        .expect("fixture proc should exist")
}

fn run(source: &str, name: &str) -> GenericValue {
    let (tree, module) = compile(source);
    Runtime::default()
        .run(&tree, &module, proc(&tree, name), None, Vec::new(), Limits::default())
        .expect("fixture should execute")
}

#[test]
fn executes_arithmetic_direct_calls_and_defaults() {
    assert_eq!(
        run(
            r#"
/proc/add(a, b = 4)
    return a + b
/proc/test()
    return add(3)
"#,
            "test",
        ),
        7.into()
    );
}

#[test]
fn nonconstant_defaults_can_read_earlier_parameters() {
    assert_eq!(
        run(
            r#"
/proc/value(a = 2, b = a + 3)
    return b
/proc/test()
    return value(4) * 10 + value()
"#,
            "test",
        ),
        75.into()
    );
}

#[test]
fn executes_phi_lowered_fibonacci_loop() {
    assert_eq!(
        run(
            r#"
/proc/fib(n)
    var/a = 0
    var/b = 1
    for(var/i = 0; i < n; i++)
        var/tmp = b
        b = a + b
        a = tmp
    return a
/proc/test()
    return fib(10)
"#,
            "test",
        ),
        55.into()
    );
}

#[test]
fn associative_iteration_preserves_keys_and_values() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/values = list("a" = 2, "b" = 5)
    var/total = 0
    for(var/key, var/value in values)
        total += values[key] + value
    return total
"#,
            "test",
        ),
        14.into()
    );
}

#[test]
fn descending_ranges_use_the_steps_sign() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/total = 0
    for(var/i = 5 to 1 step -2)
        total += i
    return total
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn virtual_dispatch_super_dot_and_defaults() {
    assert_eq!(
        run(
            r#"
/datum/base
    proc/value(a = 3)
        return a * 2
/datum/base/child
    value(a = 4)
        . = ..()
        . += 1
/proc/test()
    var/datum/base/child/object = new
    return object.value()
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn sandbox_faults_are_not_catchable() {
    let (tree, module) = compile(
        r#"
/proc/test()
    try
        shell("echo no")
    catch
        return 1
    return 0
"#,
    );
    let fault = Runtime::default()
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect_err("sandbox operation should fault");
    assert_eq!(fault.kind, FaultKind::Blocked("shell".into()));
}

#[test]
fn world_log_output_uses_a_field_reference() {
    let (tree, module) = compile(
        r#"
/proc/test()
    world.log << "hello"
    return world.log
"#,
    );
    let mut runtime = Runtime::default();
    let result = runtime
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect("fixture should execute");

    assert_eq!(result, GenericValue::Null);
    assert_eq!(runtime.output(), &[String::from("hello")]);
}

#[test]
fn unsupported_output_targets_are_blocked() {
    let (tree, module) = compile(
        r#"
/proc/test()
    null << 1
"#,
    );
    let fault = Runtime::default()
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect_err("unsupported output target should fault");

    assert_eq!(fault.kind, FaultKind::Blocked("output".into()));
}

#[test]
fn instruction_budget_stops_infinite_control_flow() {
    let (tree, module) = compile(
        r#"
/proc/test()
    while(1)
        . = 1
"#,
    );
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
            None,
            Vec::new(),
            Limits {
                instruction_budget: 20,
                ..Limits::default()
            },
        )
        .expect_err("instruction budget should stop the loop");
    assert_eq!(fault.kind, FaultKind::InstructionBudget);
}

#[test]
fn dm_throw_is_caught_with_its_value() {
    assert_eq!(
        run(
            r#"
/proc/raiser()
    throw 42
/proc/test()
    try
        raiser()
    catch(var/value)
        return value + pick(list(7))
"#,
            "test",
        ),
        49.into()
    );
}

#[test]
fn values_live_across_try_and_catch_merge_correctly() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 1
    try
        value = 2
        throw 9
    catch(var/error)
        value += error
    return value
"#,
            "test",
        ),
        11.into()
    );
}

#[test]
fn try_inside_a_loop_preserves_frame_locals() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/values = list(1, 2, 3)
    var/total = 0
    for(var/value in values)
        var/current = value
        try
            total += current
        catch
            total = -100
        current = 100
    return total
"#,
            "test",
        ),
        6.into()
    );
}

#[test]
fn parameter_defaults_are_applied_before_try_frame_storage() {
    assert_eq!(
        run(
            r#"
/proc/value(number = 4)
    try
        number += 1
    catch
        number = -100
    return number
/proc/test()
    return value()
"#,
            "test",
        ),
        5.into()
    );
}

#[test]
fn same_type_super_observes_reassigned_arguments() {
    assert_eq!(
        run(
            r#"
/datum/test/proc/value(a = 1)
    return a
/datum/test/value(a = 2)
    args[1] += 3
    a += 4
    return ..()
/proc/test()
    var/datum/test/object = new
    return object.value()
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn static_state_is_shared_and_dynamic_dm_calls_work() {
    assert_eq!(
        run(
            r#"
/datum/test
    var/static/list/items = list()
    proc/value(a = 9, b = 2)
        var/static/count = 0
        count++
        items += count
        return a + b + length(items)
/proc/test()
    var/datum/test/a = new
    var/datum/test/b = new
    a.value()
    return call(b, "value")(, 3)
"#,
            "test",
        ),
        14.into()
    );
}

#[test]
fn named_and_arglist_arguments_keep_their_layout() {
    assert_eq!(
        run(
            r#"
/proc/value(a, b, c)
    return a * 100 + b * 10 + c
/proc/forward(list/arguments)
    return value(arglist(arguments))
/proc/test()
    return forward(list("c" = 3, "a" = 1, "b" = 2))
"#,
            "test",
        ),
        123.into()
    );
}

#[test]
fn positional_arguments_fill_parameters_left_open_by_named_arguments() {
    assert_eq!(
        run(
            r#"
/proc/value(a, b, c)
    return a * 100 + b * 10 + c
/proc/test()
    return value(c = 3, 1, 2)
"#,
            "test",
        ),
        123.into()
    );
}

#[test]
fn args_is_a_dm_list_with_the_supplied_length() {
    assert_eq!(
        run(
            r#"
/proc/value(a, b)
    return args.len + islist(args)
/proc/test()
    return value(1, 2)
"#,
            "test",
        ),
        3.into()
    );
}

#[test]
fn list_iteration_uses_a_stable_entry_snapshot() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/values = list(1, 2, 3)
    var/total = 0
    for(var/value in values)
        total += value
        values.Add(value + 10)
    return total * 10 + length(values)
"#,
            "test",
        ),
        66.into()
    );
}

#[test]
fn compound_list_operators_mutate_aliases_but_plain_operators_copy() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/original = list(1, 2)
    var/list/alias = original
    alias += 3
    var/list/copy = original + 4
    return length(original) * 100 + length(alias) * 10 + length(copy)
"#,
            "test",
        ),
        334.into()
    );
}

#[test]
fn typed_iteration_filters_the_iterated_value() {
    assert_eq!(
        run(
            r#"
/datum/base
/datum/base/wanted
/datum/other
/proc/test()
    var/list/values = list(new /datum/base/wanted, new /datum/other, new /datum/base/wanted)
    var/count = 0
    for(var/datum/base/value in values)
        count++
    return count
"#,
            "test",
        ),
        2.into()
    );
}

#[test]
fn one_argument_istype_uses_the_declared_type() {
    assert_eq!(
        run(
            r#"
/datum/base
/datum/base/child
/datum/other
/proc/test()
    var/datum/base/value = new /datum/other
    return istype(value)
"#,
            "test",
        ),
        0.into()
    );
}

#[test]
fn initial_reads_the_declaration_instead_of_the_current_field() {
    assert_eq!(
        run(
            r#"
/datum/test
    var/value = 4
/proc/test()
    var/datum/test/object = new
    object.value = 9
    return initial(object.value) + issaved(object.value)
"#,
            "test",
        ),
        5.into()
    );
}

#[test]
fn initial_of_a_local_falls_back_to_its_current_value() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 4
    value = 9
    return initial(value)
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn safe_calls_on_null_return_null() {
    assert_eq!(
        run(
            r#"
/datum/test
    proc/value()
        return 1
/proc/test()
    var/datum/test/object
    return object?.value()
"#,
            "test",
        ),
        GenericValue::Null
    );
}

#[test]
fn safe_index_writes_on_null_are_discarded() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/value
    value?[1] = 2
    return 3
"#,
            "test",
        ),
        3.into()
    );
}

#[test]
fn call_depth_stops_recursive_functions() {
    let (tree, module) = compile("/proc/test()\n    return test()\n");
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
            None,
            Vec::new(),
            Limits {
                call_depth: 4,
                ..Limits::default()
            },
        )
        .expect_err("call depth should stop recursion");
    assert_eq!(fault.kind, FaultKind::CallDepth);
}

#[test]
fn an_instruction_fault_rolls_back_heap_changes() {
    let (tree, module) = compile(
        r#"
/datum
    var/value = 1
    var/list/items = list("old")
/proc/test(datum/target)
    target.value = 2
    target.items.Add("new")
    new /datum
    while(1)
        target.value++
"#,
    );
    let mut runtime = Runtime::default();
    let object = runtime
        .heap
        .alloc_object(Object::new(
            tree.id_of(&TreePath::parse("/datum")).expect("type should exist"),
        ))
        .expect("object should allocate");
    let fault = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
            None,
            vec![GenericValue::Object(object)],
            Limits {
                instruction_budget: 100,
                ..Limits::default()
            },
        )
        .expect_err("instruction budget should stop the loop");
    assert_eq!(fault.kind, FaultKind::InstructionBudget);
    assert!(
        runtime
            .heap
            .object(object)
            .expect("object should survive")
            .vars
            .is_empty()
    );
    // The fixture's datum plus the global and world singletons, both allocated ahead of the journal.
    assert_eq!(runtime.heap.objects().count(), 3);
    assert!(runtime.heap.object(ObjectId(1)).is_some());
}

#[test]
fn evaluation_reports_randomness_and_nonlocal_world_reads() {
    let (tree, module) = compile(
        r#"
/datum
/proc/test()
    rand()
    locate(/datum)
"#,
    );
    let mut runtime = Runtime::default();
    runtime.global = Some(
        runtime
            .heap
            .alloc_object(Object::new(TypeId::ROOT))
            .expect("global should allocate"),
    );
    let mut evaluator = Evaluator::new(&mut runtime, &tree, &module, Limits::default(), None);
    evaluator
        .call(proc(&tree, "test"), None, Vec::new())
        .expect("fixture should execute");
    assert!(evaluator.position_sensitive);
    assert!(!evaluator.memo_safe);
}

#[test]
fn intrinsic_procs_run_in_rust_instead_of_their_body() {
    let (tree, module) = compile(
        r#"
/world
    proc/file2list(File, Separator)
        set __demir_intrin = 122
    proc/IsBanned(key, address, computer_id, type)
        set __demir_intrin = 112
    proc/Reboot(reason)
        set __demir_intrin = 104
/proc/lines()
    return world.file2list("tips.txt")
/proc/banned()
    return world.IsBanned("key")
/proc/reboot()
    return world.Reboot()
"#,
    );

    let root = std::env::temp_dir().join(format!("dmed-intrinsic-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp dir");
    std::fs::write(root.join("tips.txt"), "first\nsecond\n").expect("fixture file");

    let mut runtime = Runtime::default();
    runtime.world.root = Some(root.clone());

    let value = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "lines"),
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect("file2list should execute");
    let GenericValue::List(id) = value else {
        panic!("file2list should return a list, got {value:?}");
    };
    let entries = runtime
        .heap
        .list(id)
        .expect("list")
        .entries
        .iter()
        .map(|(value, _)| value.display())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["first", "second", ""]);

    assert_eq!(
        runtime
            .run(
                &tree,
                &module,
                proc(&tree, "banned"),
                None,
                Vec::new(),
                Limits::default()
            )
            .expect("IsBanned should execute"),
        false.into()
    );

    let fault = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "reboot"),
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("Reboot should be blocked");
    assert_eq!(fault.kind, FaultKind::Blocked("world.Reboot".into()));

    std::fs::remove_dir_all(&root).ok();
}

/// Without a root there is no filesystem to reach, rather than an ambient one.
#[test]
fn file2list_without_a_root_is_blocked() {
    let (tree, module) = compile(
        r#"
/world
    proc/file2list(File, Separator)
        set __demir_intrin = 122
/proc/lines()
    return world.file2list("tips.txt")
"#,
    );

    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, "lines"),
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("file2list should be blocked");
    assert_eq!(fault.kind, FaultKind::Blocked("filesystem".into()));
}

/// `/world` procs run with the world as `src`, so assignments land on it rather than on globals.
#[test]
fn world_vars_are_readable_and_writable_through_world_procs() {
    assert_eq!(
        run(
            r#"
/world
    var/booted = 0
    proc/boot()
        booted = 7
/proc/test()
    world.boot()
    return world.booted
"#,
            "test",
        ),
        7.into()
    );
}

/// `new /image(...)` has to fill the object `new` already allocated. An intrinsic constructor that
/// allocated its own would leave the caller holding an empty one.
#[test]
fn image_constructor_fills_the_object_new_allocated() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/image/I = new('thing.dmi', null, "state")
    return I.icon_state
"#,
            "test",
        ),
        "state".into()
    );
}

/// `matrix(M, ...)` is the in-place form every `/matrix` method in `stddef.dm` routes through.
/// Transforms are unimplemented, so it must hand the matrix back rather than write the argument
/// list onto `a`/`b`/`c`.
#[test]
fn matrix_in_place_form_leaves_the_matrix_alone() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/matrix/M = new(2)
    return M.a
"#,
            "test",
        ),
        1.into()
    );
}

/// `/alist` is a list, not a plain object, so `new` has to take the same path `/list` does.
#[test]
fn new_alist_is_a_list_seeded_from_its_pairs() {
    assert_eq!(
        run(
            r#"
/alist
    var/len
    proc/New(items)
/proc/islist(L)
    set __demir_intrin = 279
/proc/test()
    var/alist/A = new(list("a", "b"))
    return islist(A) + A.len
"#,
            "test",
        ),
        3.into()
    );
}

/// `/sound/New` copies its file into the resource cache first, so a blocked `fcopy_rsc` would make
/// every `new /sound(...)` fault.
#[test]
fn sound_constructor_keeps_its_file() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/sound/S = new('beep.ogg')
    return S.file
"#,
            "test",
        ),
        GenericValue::Resource("beep.ogg".into())
    );
}

/// `PROC_REF(X)` is `nameof(.proc/X)`, and tgstation spells almost every callback that way.
#[test]
fn nameof_resolves_a_proc_path_to_its_name() {
    assert_eq!(
        run(
            r#"
/proc/nameof(X)
    set __demir_intrin = 400
/proc/work()
    return 1
/proc/test()
    return nameof(/proc/work)
"#,
            "test",
        ),
        "work".into()
    );
}

/// A global proc called from inside a method binds to the global. `find_proc` would reach the same
/// body, but only after failing a lookup on the runtime type of `src` at every call.
#[test]
fn a_global_proc_called_from_a_method_reaches_the_global() {
    assert_eq!(
        run(
            r#"
/proc/helper(n)
    return n * 2
/datum/thing
    proc/work()
        return helper(21)
/proc/test()
    var/datum/thing/T = new
    return T.work()
"#,
            "test",
        ),
        42.into()
    );
}

/// When a type declares the same name, the method wins and the call has to stay dynamic, or a
/// subtype's override would be linked away.
#[test]
fn a_method_of_the_same_name_still_shadows_the_global() {
    assert_eq!(
        run(
            r#"
/proc/helper(n)
    return 1
/datum/thing
    proc/helper(n)
        return 2
    proc/work()
        return helper(0)
/datum/thing/special
    helper(n)
        return 3
/proc/test()
    var/datum/thing/T = new /datum/thing/special
    return T.work()
"#,
            "test",
        ),
        3.into()
    );
}

/// The positional argument names come from the prelude's own signature, so `image()`'s last two
/// parameters land even though nothing in the VM lists them.
#[test]
fn appearance_arguments_follow_the_declared_signature() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/image/I = image('a.dmi', null, "s", 3, 1, 4, 5, 6, 7)
    return I.pixel_w * 10 + I.pixel_z
"#,
            "test",
        ),
        67.into()
    );
}

/// `mutable_appearance(appearance)` copies an appearance onto the new one, which is what BYOND's
/// signature says and what the prelude declares. The old hardcoded list read that argument as
/// `icon` instead.
#[test]
fn mutable_appearance_takes_an_appearance() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/image/source = new
    source.icon_state = "src"
    var/mutable_appearance/MA = mutable_appearance(source)
    return MA.icon_state
"#,
            "test",
        ),
        "src".into()
    );
}

/// `RemoveAll` drops every occurrence and answers how many went, which is how `list_clear_nulls`
/// on a tgstation downstream asks whether the list held any nulls.
#[test]
fn list_remove_all_reports_how_many_it_dropped() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/L = list(1, null, 2, null, null, 3)
    var/dropped = L.RemoveAll(null)
    return dropped * 100 + L.len * 10 + L[1]
"#,
            "test",
        ),
        331.into()
    );
}

/// `Splice(Start, End, Item...)` cuts the range and drops the items in its place.
#[test]
fn list_splice_replaces_a_range() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/L = list("a", "b", "c")
    L.Splice(2, 3, "x", "y")
    return L.Join("")
"#,
            "test",
        ),
        "axyc".into()
    );
}

/// List methods use the object tree like datum methods. A user declaration replaces the prelude
/// intrinsic and receives the list itself as `src`.
#[test]
fn list_methods_are_overridable_dm_procs_with_list_src() {
    assert_eq!(
        run(
            r#"
/list/Add(Item1)
    return src.len * 10 + Item1
/proc/test()
    var/list/L = list(1, 2)
    return L.Add(7) * 10 + L.len
"#,
            "test",
        ),
        272.into()
    );
}

/// The prelude declaration remains in the normal override chain, so `..()` reaches the intrinsic
/// implementation and keeps the original list receiver.
#[test]
fn list_method_overrides_can_call_the_intrinsic_super_proc() {
    assert_eq!(
        run(
            r#"
/list/Add(Item1)
    ..()
    return src.len
/proc/test()
    var/list/L = list(1, 2)
    return L.Add(7) * 10 + L.len
"#,
            "test",
        ),
        33.into()
    );
}

/// `/alist` has its own runtime kind but inherits the ordinary `/list` proc declarations.
#[test]
fn alists_inherit_list_intrinsics_through_proc_dispatch() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/alist/A = alist("first")
    A.Add("value")
    return istype(A, /alist) * 10 + A.len
"#,
            "test",
        ),
        12.into()
    );
}
