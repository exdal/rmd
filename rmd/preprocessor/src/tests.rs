macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
    };
}

use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::*;

fn pp(files: &[(&str, &str)]) -> String {
    let (rendered, _) = pp_with_errors(files);

    rendered
}

/// [`pp`], also handing back what the run complained about.
fn pp_with_errors(files: &[(&str, impl AsRef<[u8]>)]) -> (String, Vec<String>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-pp-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");

    for (name, source) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("source directory")).expect("source directory");
        fs::write(path, source).expect("write source");
    }

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .run(dir.join(files[0].0))
        .expect("preprocess");
    let rendered = render(&result.tokens);
    let errors = result.errors.iter().map(ToString::to_string).collect::<Vec<_>>();
    let _ = fs::remove_dir_all(&dir);

    (rendered, errors)
}

fn indent_errors(source: &str) -> Vec<String> {
    let (_, errors) = pp_with_errors(&[("a.dm", source)]);

    errors
        .into_iter()
        .filter(|error| error.contains("inconsistent indentation"))
        .collect()
}

#[test]
fn nested_pragmas_restore_overrides_and_defaults() {
    let (_, errors) = pp_with_errors(&[(
        "a.dm",
        "#pragma push
#pragma UndefineMissingDirective notice
#undef OUTER
#pragma push
#pragma ignore 1101
#undef HIDDEN
#pragma push
#pragma error UndefineMissingDirective
#undef INNER
#pragma pop
#undef STILL_HIDDEN
#pragma pop
#undef RESTORED_OVERRIDE
#pragma pop
#undef RESTORED_DEFAULT
",
    )]);

    let levels = errors
        .iter()
        .map(|error| error.split(" at ").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        levels,
        ["notice DR1101", "error DR1101", "notice DR1101", "warning DR1101"]
    );
}

#[test]
fn byond_warning_controls_accept_lists_and_compatibility_names() {
    let (rendered, errors) = pp_with_errors(&[(
        "a.dm",
        "#pragma ignore UndefineMissingDirective, WarningDirective, unused_var
#undef HIDDEN
#warn hidden
#pragma error 1101, 1201, init_proc
#undef AS_ERROR
#warn as error
#pragma warn UndefineMissingDirective, WarningDirective, unused_var
#undef AS_WARNING
#warn as warning
#pragma ignore unused_var, init_proc
var/kept = 1
",
    )]);

    let levels = errors
        .iter()
        .map(|error| error.split(" at ").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        levels,
        ["error DR1101", "error DR1201", "warning DR1101", "warning DR1201"]
    );
    assert!(rendered.contains("var / kept = 1"), "{rendered}");
    assert!(!rendered.contains("unused_var"), "{rendered}");
}

#[test]
fn pragma_state_spans_include_boundaries_without_a_trailing_newline() {
    let (_, errors) = pp_with_errors(&[
        (
            "a.dm",
            "#pragma push
#pragma UndefineMissingDirective notice
#include \"push.dm\"
#undef HIDDEN
#include \"pop.dm\"
#undef RESTORED
#pragma pop
#undef DEFAULT
",
        ),
        (
            "push.dm",
            "#pragma push\n#pragma ignore UndefineMissingDirective\n#pragma push",
        ),
        ("pop.dm", "#pragma pop\n#pragma pop"),
    ]);

    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].starts_with("notice DR1101"), "{errors:?}");
    assert!(errors[1].starts_with("warning DR1101"), "{errors:?}");
}

#[test]
fn unmatched_pragma_pop_preserves_state_and_unmatched_push_is_allowed() {
    let (_, errors) = pp_with_errors(&[(
        "a.dm",
        "#pragma ignore UndefineMissingDirective
#pragma pop
#undef HIDDEN
#pragma push",
    )]);

    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("#pragma pop without a matching #pragma push"),
        "{errors:?}"
    );
}

#[test]
fn inactive_branches_do_not_change_pragma_state() {
    let (_, errors) = pp_with_errors(&[(
        "a.dm",
        "#pragma push
#pragma ignore UndefineMissingDirective
#if 0
#pragma pop
#pragma error UndefineMissingDirective
#pragma push
#pragma ignored garbage
#endif
#undef HIDDEN
#pragma pop
#undef VISIBLE
",
    )]);

    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("warning DR1101"), "{errors:?}");
    assert!(errors[0].contains("VISIBLE"), "{errors:?}");
}

#[test]
fn legacy_pragma_levels_and_aliases_still_work() {
    for (level, expected) in [
        ("disabled", None),
        ("disable", None),
        ("notice", Some("notice")),
        ("pedantic", Some("notice")),
        ("info", Some("notice")),
        ("warning", Some("warning")),
        ("warn", Some("warning")),
        ("error", Some("error")),
        ("err", Some("error")),
        ("WARNING", Some("warning")),
    ] {
        let source = format!("#pragma 1101 {level}\n#undef PROBE\n");
        let (_, errors) = pp_with_errors(&[("a.dm", source)]);

        if let Some(expected) = expected {
            assert_eq!(errors.len(), 1, "{level}: {errors:?}");
            assert!(
                errors[0].starts_with(&format!("{expected} DR1101")),
                "{level}: {errors:?}"
            );
        } else {
            assert!(errors.is_empty(), "{level}: {errors:?}");
        }
    }
}

#[test]
fn unknown_legacy_warning_names_and_numbers_still_warn() {
    for directive in ["unused_var disabled", "65000 disabled", "ignore 65000"] {
        let source = format!("#pragma {directive}\nvar/kept = 1\n");
        let (rendered, errors) = pp_with_errors(&[("a.dm", source)]);

        assert_eq!(errors.len(), 1, "{directive}: {errors:?}");
        assert!(errors[0].starts_with("warning DR1002"), "{directive}: {errors:?}");
        assert!(rendered.contains("var / kept = 1"), "{rendered}");
    }
}

#[test]
fn pragma_controls_cannot_change_fatal_codes() {
    for directive in [
        "BadDirective disabled",
        "ignore BadDirective",
        "warn 10",
        "error BadToken",
    ] {
        let source = format!("#pragma {directive}\n#pragma pop\n");
        let (_, errors) = pp_with_errors(&[("a.dm", source)]);

        assert_eq!(errors.len(), 2, "{directive}: {errors:?}");
        assert!(
            errors.iter().all(|error| error.starts_with("error DR0010")),
            "{errors:?}"
        );
        assert!(errors[0].contains("must always be an error"), "{errors:?}");
    }
}

#[test]
fn malformed_pragmas_do_not_consume_the_following_line_or_emit_arguments() {
    for directive in [
        "",
        "push extra",
        "pop extra",
        "ignore",
        "warn",
        "error",
        "ignore unused_var,",
        "ignore ,unused_var",
        "warn unused_var,,init_proc",
        "error unused_var init_proc",
        "ignore \"unused_var\"",
        "UndefineMissingDirective",
        "UndefineMissingDirective bogus",
        "UndefineMissingDirective disabled extra",
        "\"invalid\" disabled",
    ] {
        let source = format!("#pragma {directive}\nvar/kept = 1\n#undef PROBE\n");
        let (rendered, errors) = pp_with_errors(&[("a.dm", source)]);

        assert_eq!(errors.len(), 2, "{directive}: {errors:?}");
        assert!(errors[0].starts_with("error DR0010"), "{directive}: {errors:?}");
        assert!(errors[0].contains("at 1:"), "{directive}: {errors:?}");
        assert!(errors[1].starts_with("warning DR1101"), "{directive}: {errors:?}");
        assert_eq!(rendered.trim(), "var / kept = 1", "{directive}: {rendered}");
    }
}

#[test]
fn pragma_comments_continuations_and_eof_are_supported() {
    let (rendered, errors) = pp_with_errors(&[(
        "a.dm",
        "#pragma push // save state
#pragma ignore /* warnings */ UndefineMissingDirective, \\
WarningDirective // continued list
#undef HIDDEN
#warn hidden
var/kept = 1
#pragma pop /* restore state */",
    )]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered.trim(), "var / kept = 1", "{rendered}");

    for directive in ["", "ignore", "ignore unused_var,", "UndefineMissingDirective"] {
        let (_, errors) = pp_with_errors(&[("a.dm", format!("#pragma {directive}"))]);
        assert_eq!(errors.len(), 1, "{directive}: {errors:?}");
        assert!(errors[0].starts_with("error DR0010"), "{directive}: {errors:?}");
    }
}

#[test]
fn indented_byond_pragmas_preserve_surrounding_code_layout() {
    let source = "proc/example()
\tvar/before = 1
\t#ifndef SPACEMAN_DMM
\t#pragma push
\t#pragma ignore unused_var // BYOND compatibility
\tvar/our_oven = 2
\tour_oven = before
\t#pragma pop
\t#endif
\treturn our_oven
var/after = 3
";
    let (rendered, errors) = pp_with_errors(&[("a.dm", source)]);
    let expected = pp(&[(
        "a.dm",
        "proc/example()\n\tvar/before = 1\n\tvar/our_oven = 2\n\tour_oven = before\n\treturn our_oven\nvar/after \
         = 3\n",
    )]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered, expected);
}

#[test]
fn stddef_true_and_false_survive_as_tokens() {
    let source = fixture!("programs/stddef_true_and_false_survive_as_tokens.dm");
    let (rendered, errors) = pp_with_errors(&[("a.dm", source)]);

    assert!(errors.is_empty(), "{errors:?}");
    assert!(rendered.contains("yes"), "{rendered}");
    assert!(!rendered.contains("no"), "{rendered}");
    assert!(rendered.contains("defined"), "{rendered}");
    assert!(rendered.contains("x = TRUE"), "{rendered}");
}

#[test]
fn a_codebase_can_still_define_its_own_true() {
    let source = fixture!("programs/a_codebase_can_still_define_its_own_true.dm");
    let (rendered, _) = pp_with_errors(&[("a.dm", source)]);

    assert!(rendered.contains("x = 2"), "{rendered}");
}

#[test]
fn an_indented_directive_is_not_part_of_the_code_indentation() {
    // The `#endif` lines up with the `#if`, not with anything the code opened.
    assert_eq!(
        indent_errors(fixture!("programs/indented-directive-off.dm")),
        Vec::<String>::new()
    );
    assert_eq!(
        indent_errors(fixture!("programs/indented-directive-on.dm")),
        Vec::<String>::new()
    );
}

#[test]
fn a_conditional_compiled_out_mid_block_leaves_the_indentation_alone() {
    let source = fixture!("programs/conditional-mid-block.dm");

    assert_eq!(indent_errors(source), Vec::<String>::new());
}

#[test]
fn a_real_indentation_mistake_is_still_reported_around_directives() {
    // The dedent lands between two open blocks, which no directive explains away.
    assert_eq!(indent_errors(fixture!("programs/real-indentation-error.dm")).len(), 1);
    assert_eq!(
        indent_errors(fixture!("programs/real-indentation-error-with-directive.dm")).len(),
        1
    );
}

#[test]
fn macro_generated_defines_preserve_function_like_spacing() {
    let rendered = pp(&[("generated.dm", fixture!("programs/macro-generated-defines.dm"))]);

    assert_eq!(rendered, "FUNCTION\nvalue\n( x ) x");
}

#[test]
fn concat_pastes_fixed_text_onto_a_parameter() {
    let rendered = pp(&[("fixed_concat.dm", fixture!("programs/fixed-concat.dm"))]);

    assert_eq!(rendered, "src . body_standing");
}

#[test]
fn a_bare_generated_function_macro_survives_in_a_nested_path_macro() {
    let rendered = pp(&[("nested.dm", fixture!("programs/nested-path-macro.dm"))]);

    assert_eq!(rendered, "/ datum / base / OFFSETS { } / datum / other / OFFSETS { }");
}

#[test]
fn a_pasted_identity_macro_captures_a_following_argument_list() {
    let direct = pp(&[("identity.dm", fixture!("programs/identity-macro.dm"))]);
    let nested = pp(&[("nested_identity.dm", fixture!("programs/nested-identity-macro.dm"))]);

    assert_eq!(direct, "datum / namespace / CHEM / var / const / REQUEST = 1");
    assert_eq!(
        nested,
        "datum / namespace / CHEM / var / datum / namespace / inner / REQUEST = 1"
    );
}

#[test]
fn recursive_macros_stop_at_the_token_that_already_expanded_them() {
    let rendered = pp(&[("recursive.dm", fixture!("programs/recursive-macros.dm"))]);

    assert_eq!(rendered, "SELF\nLEFT");
}

#[test]
fn a_condition_inside_parentheses_ends_at_its_physical_line() {
    let rendered = pp(&[("conditional_list.dm", fixture!("programs/conditional-list.dm"))]);

    assert_eq!(
        rendered,
        "var / list / items = list ( first , conditional , last , )\nvar / after = 1"
    );
}

#[test]
fn an_inactive_branch_does_not_close_the_surrounding_expression() {
    let rendered = pp(&[("conditional_close.dm", fixture!("programs/conditional-close.dm"))]);

    assert_eq!(rendered, "var / items = list ( first , )\nvar / after = 1");
}

#[test]
fn fexists_checks_literal_paths_and_guards_optional_includes() {
    let (rendered, errors) = pp_with_errors(&[
        ("entry.dm", fixture!("programs/fexists-literals.dm")),
        ("+secret/foo.dm", "optional_file\n"),
    ]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered, "optional_file\nmissing_file\nbackslash_path\nnot_a_file");
}

#[test]
fn fexists_expands_macro_arguments_in_if_and_elif() {
    let (rendered, errors) = pp_with_errors(&[
        ("entry.dm", fixture!("programs/fexists-macros.dm")),
        ("+secret/foo.dm", ""),
    ]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        rendered,
        "object_macro\nchained_macro\nfunction_macro\nwrapped_fexists\nnegated_combination\nelif_macro"
    );
}

#[test]
fn fexists_resolves_macro_paths_relative_to_each_including_file() {
    let (rendered, errors) = pp_with_errors(&[
        ("entry.dm", fixture!("programs/fexists-outer.dm")),
        ("nested/inner.dm", fixture!("programs/fexists-inner.dm")),
        ("nested/local.dm", "nested_file\n"),
        ("root-only.dm", ""),
    ]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered, "nested_file\nrestored_directory");
}

#[test]
fn fexists_still_diagnoses_macros_that_do_not_expand_to_a_path() {
    let (rendered, errors) = pp_with_errors(&[("entry.dm", fixture!("programs/fexists-invalid-macro.dm"))]);

    assert_eq!(rendered, "invalid_path_rejected");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("fexists() expects a file path"), "{errors:?}");
}

#[test]
fn indented_dead_branch_keeps_layout() {
    let flat = pp(&[("flat.dm", fixture!("programs/dead-branch-flat.dm"))]);
    let indented = pp(&[("indented.dm", fixture!("programs/dead-branch-indented.dm"))]);

    assert_eq!(flat, indented);
    assert_eq!(flat, "/ datum / a\n/ datum / b\n> var / y = 2\n<");
}

#[test]
fn an_include_is_indented_against_its_includer() {
    let spliced = pp(&[
        ("outer.dm", fixture!("programs/include-indented-outer.dm")),
        ("inner.dm", fixture!("programs/include-indented-inner.dm")),
    ]);

    assert_eq!(
        spliced,
        "/ datum / a\n> var / x = 1\n< / datum / inc\n> var / z = 3\n< / datum / b"
    );
}

#[test]
fn an_include_hands_its_open_blocks_back() {
    let trailing = pp(&[
        ("outer.dm", fixture!("programs/include-open-block-outer.dm")),
        ("tail.dm", fixture!("programs/include-open-block-tail.dm")),
    ]);
    assert_eq!(trailing, "/ datum / a\n> var / x = 1\nvar / deep = 4\n< / datum / b");

    let empty = pp(&[
        ("block.dm", fixture!("programs/include-empty-outer.dm")),
        ("nothing.dm", ""),
    ]);
    assert_eq!(empty, "/ datum / a\n> var / x = 1\nvar / y = 2\n<");
}

#[test]
fn includes_ignore_case_like_windows_byond() {
    let (rendered, errors) = pp_with_errors(&[
        ("entry.dm", fixture!("programs/include-ignoring-case.dm")),
        ("maps/station.dm", "station_file\n"),
    ]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered, "station_file");
}

#[test]
fn dms_includes_are_ignored() {
    let (rendered, errors) = pp_with_errors(&[
        ("entry.dm", fixture!("programs/include-dms.dm")),
        ("style.dms", "body { color: red; }\n"),
    ]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered, "/ datum / a\n/ datum / b");
}

#[test]
fn only_files_with_pragma_multiple_are_included_again() {
    let (rendered, errors) = pp_with_errors(&[
        ("entry.dm", fixture!("programs/pragma-multiple-entry.dm")),
        ("part.dm", fixture!("programs/pragma-multiple-part.dm")),
        ("once.dm", fixture!("programs/pragma-multiple-once.dm")),
    ]);

    assert_eq!(rendered, "repeated\nsingle\nrepeated");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("once.dm\" was already included"), "{errors:?}");
}

#[test]
fn a_file_including_itself_stops_at_the_depth_limit() {
    let (_, errors) = pp_with_errors(&[("self.dm", fixture!("programs/pragma-multiple-self.dm"))]);

    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("nested too deeply"), "{errors:?}");
}

#[test]
fn a_brace_owns_the_indent_it_opens() {
    let rendered = pp(&[("writer.dm", fixture!("programs/brace-owned-indent.dm"))]);

    assert_eq!(
        rendered,
        "dmm_writer {\nproc {\nmake ( x ) {\nif ( x ) {\na ( )\n} else { b ( ) }\nif ( x )\n> c ( )\n< \
         return\n}\n}\n}\n/ datum / after"
    );
}

#[test]
fn source_text_can_be_shared_between_configuration_passes() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("demir-source-cache-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let entry = dir.join("entry.dm");
    fs::write(&entry, fixture!("programs/source-cache-first.dm")).expect("first source");

    let arena = StrArena::new();
    let first = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .run(&entry)
        .expect("first pass");
    let cache = first.source_cache.clone();
    assert_eq!(cache.len(), 1);

    fs::write(&entry, fixture!("programs/source-cache-second.dm")).expect("changed source");
    let second = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .with_source_cache(cache)
        .run(&entry)
        .expect("second pass");
    fs::remove_dir_all(&dir).expect("remove temp dir");

    assert!(render(&second.tokens).contains("x = 1"));
    assert!(!render(&second.tokens).contains("x = 2"));
}

#[test]
fn a_utf8_file_can_include_latin1_source() {
    let (rendered, errors) = pp_with_errors(&[
        (
            "entry.dme",
            "var/utf8 = \"café 日本 🦀\"\n#include \"legacy.dm\"\n".as_bytes(),
        ),
        (
            "legacy.dm",
            b"// vector: 90\xb0\r\nvar/legacy = \"caf\xe9 \xb0 \x92\"\r\n".as_slice(),
        ),
    ]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        rendered,
        "var / utf8 = \"café 日本 🦀\"\nvar / legacy = \"café ° \u{0092}\""
    );
}

#[test]
fn an_entry_file_can_be_latin1() {
    let (rendered, errors) = pp_with_errors(&[("entry.dm", b"var/name = \"caf\xe9\"\n".as_slice())]);

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rendered, "var / name = \"café\"");
}

#[test]
fn decoded_latin1_source_is_cached_between_configuration_passes() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-latin1-source-cache-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let entry = dir.join("entry.dme");
    let legacy = dir.join("legacy.dm");
    fs::write(&entry, "#include \"legacy.dm\"\n").expect("write entry");
    fs::write(&legacy, b"var/name = \"caf\xe9\"\n").expect("write Latin-1 source");

    let arena = StrArena::new();
    let first = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .run(&entry)
        .expect("first pass");
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert_eq!(first.source_cache.len(), 2);
    let file = first.sources.find(&legacy).expect("included source");
    let contents = first.sources.contents(file).expect("decoded source");
    assert_eq!(contents, "var/name = \"café\"\n");

    fs::remove_file(&legacy).expect("remove Latin-1 source");
    let second = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .with_source_cache(first.source_cache.clone())
        .run(&entry)
        .expect("second pass");
    fs::remove_dir_all(&dir).expect("remove temp dir");

    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert_eq!(render(&second.tokens), "var / name = \"café\"");
    let file = second.sources.find(&legacy).expect("cached source");
    let cached = second.sources.contents(file).expect("cached contents");
    assert!(std::ptr::eq(contents, cached));
}

#[test]
fn a_missing_include_still_reports_a_missing_file() {
    let (rendered, errors) = pp_with_errors(&[("entry.dm", "#include \"missing.dm\"\nvar/x = 1\n")]);

    assert_eq!(rendered, "var / x = 1");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("could not find included file"), "{errors:?}");
    assert!(errors[0].contains("missing.dm"), "{errors:?}");
}

#[test]
fn a_macro_defined_after_its_use_is_reported_at_the_use() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-macro-order-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let entry = dir.join("entry.dm");
    fs::write(&entry, fixture!("programs/macro-used-before-define.dm")).expect("write entry");

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .run(&entry)
        .expect("preprocess");
    let uses = result.tokens.iter().filter_map(|(token, location)| match token {
        Token::Identifier(name) => Some((*name, *location)),
        _ => None,
    });
    let errors = macros_used_before_definition(&result.defines, &result.sources, &dir, uses);
    fs::remove_dir_all(&dir).expect("remove temp dir");

    let errors = errors.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(
        errors,
        [
            "error DR0404 at 2:9: 'LATER' is used before its #define at entry.dm:4:1, macros only apply after \
             they are defined"
        ]
    );
}

#[test]
fn an_included_file_cannot_leak_a_peeked_eof() {
    let rendered = pp(&[
        ("outer.dm", fixture!("programs/include-eof-outer.dm")),
        ("tail.dm", fixture!("programs/include-eof-tail.dm")),
    ]);

    assert_eq!(rendered, "/ datum / a\n> var / x = 1\nvar / deep = LAST\n< / datum / b");
}

/// `stddef.dm`, then `demir.dm`, then the entry. The second prelude file may lean on the first.
#[test]
fn the_prelude_runs_in_order_ahead_of_the_entry() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-prelude-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");

    fs::write(dir.join("first.dm"), fixture!("programs/prelude-first.dm")).expect("write first");
    fs::write(dir.join("second.dm"), fixture!("programs/prelude-second.dm")).expect("write second");
    fs::write(dir.join("entry.dme"), fixture!("programs/prelude-entry.dme")).expect("write entry");

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .with_prelude([
            PreludeFile::Disk(dir.join("first.dm")),
            PreludeFile::Disk(dir.join("second.dm")),
        ])
        .run(dir.join("entry.dme"))
        .expect("preprocess");

    let rendered = render(&result.tokens);
    let _ = fs::remove_dir_all(&dir);

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(rendered.contains("x = 1"), "{rendered}");
}

#[test]
fn the_postlude_runs_after_the_entry_and_shares_its_defines() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-postlude-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    fs::write(dir.join("entry.dm"), fixture!("programs/postlude-entry.dm")).expect("write entry");

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .without_prelude()
        .with_prelude([PreludeFile::Embedded(
            "<before-entry.dm>",
            fixture!("programs/postlude-prelude.dm"),
        )])
        .with_postlude([PreludeFile::Embedded(
            "<after-entry.dm>",
            fixture!("programs/postlude-postlude.dm"),
        )])
        .run(dir.join("entry.dm"))
        .expect("preprocess");

    let rendered = render(&result.tokens);
    let _ = fs::remove_dir_all(&dir);

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(rendered.contains("/ datum / entry\n> var / value = 3"), "{rendered}");
    assert!(rendered.contains("/ datum / injected\n> var / value = 7"), "{rendered}");
}

#[test]
fn baking_disables_mapping_compatibility_defines() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-baking-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let entry = dir.join("entry.dm");
    // tgstation's `__byond_version_compat.dm`, which only `SPACEMAN_DMM` used to skip
    fs::write(&entry, fixture!("programs/baking-compatibility.dm")).expect("write entry");

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .with_baking(true)
        .run(&entry)
        .expect("preprocess");
    let _ = fs::remove_dir_all(&dir);

    assert!(result.is_ok(), "{:?}", result.errors);
    assert!(result.defines.is_defined("__DEMIR_BAKE__"));
    assert!(!result.defines.is_defined("FASTDMM"));
    assert!(!result.defines.is_defined("SPACEMAN_DMM"));
}

/// `#if 0` around code the lexer would reject must not diagnose the dead branch. Indentation
/// errors already ride along in `IndentState`; these are the ones that do not.
#[test]
fn inactive_branch_lex_errors_do_not_leak() {
    let dir = std::env::temp_dir().join(format!("demir-inactive-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let entry = dir.join("entry.dm");
    fs::write(&entry, fixture!("programs/inactive-branch-lex-errors.dm")).expect("write entry");

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .without_core()
        .without_prelude()
        .run(&entry)
        .expect("preprocess");
    let rendered = render(&result.tokens);
    let _ = fs::remove_dir_all(&dir);

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(!rendered.contains("ignored"), "{rendered}");
    assert!(rendered.contains("valid"), "{rendered}");
}

/// Every source is compiled in, so a shipped binary needs no prelude directory beside it.
#[test]
fn the_default_prelude_is_version_core_stddef_ext_demir_then_imgui_and_needs_no_files() {
    let names: Vec<_> = core_files()
        .into_iter()
        .chain(prelude_files())
        .map(|file| match file {
            PreludeFile::Embedded(name, _) => name,
            PreludeFile::Disk(_) => "disk",
        })
        .collect();

    assert_eq!(
        names,
        vec![
            "<version.dm>",
            "<core.dm>",
            "<stddef.dm>",
            "<stddef_ext.dm>",
            "<demir.dm>",
            "<imgui.dm>"
        ]
    );
    assert!(CORE_SOURCE.contains("/datum"));
    assert!(STDDEF_SOURCE.contains("#define NORTH 1"));
    assert!(STDDEF_EXT_SOURCE.contains("/atom"));
    assert!(DEMIR_SOURCE.contains("#define __DEMIR__"));
}

#[test]
fn disabling_the_optional_prelude_keeps_core() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rmd-core-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let entry = dir.join("entry.dm");
    fs::write(&entry, fixture!("programs/without-prelude.dm")).expect("write entry");

    let arena = StrArena::new();
    let result = Preprocessor::new(&arena)
        .without_prelude()
        .run(&entry)
        .expect("preprocess");
    let _ = fs::remove_dir_all(&dir);

    assert!(result.sources.find("<version.dm>").is_some());
    assert!(result.sources.find("<core.dm>").is_some());
    assert!(result.sources.find("<stddef.dm>").is_none());
    assert!(result.sources.find("<demir.dm>").is_none());
    assert!(result.defines.is_defined("DM_VERSION"));
    assert!(result.defines.is_defined("DM_BUILD"));
}

/// The builtins reach the tree without a single file read.
#[test]
fn the_embedded_prelude_defines_the_builtins() {
    let arena = StrArena::new();
    let mut preprocessor = Preprocessor::new(&arena);
    preprocessor.prelude = vec![
        PreludeFile::Embedded("<version.dm>", VERSION_SOURCE),
        PreludeFile::Embedded("<core.dm>", CORE_SOURCE),
        PreludeFile::Embedded("<stddef.dm>", STDDEF_SOURCE),
        PreludeFile::Embedded("<stddef_ext.dm>", STDDEF_EXT_SOURCE),
        PreludeFile::Embedded("<demir.dm>", DEMIR_SOURCE),
        PreludeFile::Embedded("<imgui.dm>", IMGUI_SOURCE),
    ];

    for file in std::mem::take(&mut preprocessor.prelude) {
        let PreludeFile::Embedded(name, contents) = file else {
            continue;
        };

        preprocessor.open_embedded(name, contents);
        preprocessor.drain();
        preprocessor.end_stream();
    }

    assert!(preprocessor.errors.is_empty(), "{:?}", preprocessor.errors);
    assert!(preprocessor.defines.is_defined("NORTH"));
    assert!(preprocessor.defines.is_defined("DM_VERSION"));
}
