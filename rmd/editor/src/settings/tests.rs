use dear_imgui_rs::Key;
use ui::keybind::{BINDABLE_KEYS, KeyBinding, MODIFIER_KEYS};

use super::*;
use super::{
    keybind::{KeybindAction, KeybindPreset},
    paths::settings_path_from,
};

#[test]
fn map_issues_panel_starts_closed_even_if_previously_saved_open() {
    let old: Settings = toml::from_str("[panels]\nsearch = true\n").unwrap();
    assert!(!old.panels.map_issues);
    let previous: Settings = toml::from_str("[panels]\nmap_issues = true\nsearch = false\n").unwrap();
    assert!(!previous.panels.map_issues);
    assert!(!previous.panels.search);
    let mut settings = old;
    settings.panels.map_issues = true;
    let encoded = toml::to_string(&settings).unwrap();
    assert!(!encoded.contains("map_issues"));
    let decoded: Settings = toml::from_str(&encoded).unwrap();
    assert!(!decoded.panels.map_issues);
}

#[test]
fn defaults_preserve_existing_editor_behavior() {
    assert_eq!(
        Settings::default(),
        Settings {
            maximized: false,
            preferred_editor: String::from(DEFAULT_PREFERRED_EDITOR),
            show_areas: false,
            show_area_outlines: true,
            show_lighting: true,
            minimum_light_brightness_percent: 0,
            focus_windows_on_hover: true,
            ui_scale_override_percent: None,
            theme: String::from(DEFAULT_THEME),
            font: FontSettings::default(),
            tile_place_flash: true,
            selection_guide_line: true,
            show_tile_grid: true,
            tile_grid_min_pixels: 10,
            show_tile_grid_axis: true,
            show_selected_pixel_grid: true,
            mirror_camera: false,
            selected_pixel_grid_min_pixels: 3,
            show_pixel_grid_axis: true,
            selection_highlight: SelectionHighlight::Outline,
            object_tree_line_indicators: true,
            object_tree_search: ObjectTreeSearchOptions::default(),
            object_tree_filter: ObjectTreeFilterOptions::default(),
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            trusted_codebases: Vec::new(),
            recent: Vec::new(),
            profile_selections: Vec::new(),
            forced_profile_selections: Vec::new(),
            optimizations_enabled: true,
            bake_enabled: true,
            git_enabled: true,
            blame_depth: 500,
            check_for_updates: true,
            sanitize_vars_on_save: false,
            pinned_vars: Vec::new(),
            pinned_types: Vec::new(),
            coop: CoopSettings::default(),
            panels: OpenPanels::default(),
            show_dm_ui: true,
        }
    );
}

#[test]
fn default_keybindings_match_the_replaced_shortcuts() {
    let bindings = KeyBindings::default();
    let expected = [
        (KeybindAction::ShowAreas, Key::A),
        (KeybindAction::ShowAreaOutlines, Key::O),
        (KeybindAction::LevelUp, Key::PageUp),
        (KeybindAction::LevelDown, Key::PageDown),
        (KeybindAction::Refit, Key::Home),
        (KeybindAction::PlaceTool, Key::W),
        (KeybindAction::PickTool, Key::S),
        (KeybindAction::MoveTool, Key::M),
        (KeybindAction::DeleteTool, Key::X),
        (KeybindAction::ReplaceTool, Key::E),
        (KeybindAction::FillTool, Key::Q),
        (KeybindAction::Rotate, Key::R),
        (KeybindAction::ShowTileGrid, Key::G),
    ];

    for (action, key) in expected {
        assert_eq!(bindings.get(action), KeyBinding::new(key));
    }
    assert_eq!(bindings.get(KeybindAction::SelectTool), KeyBinding::with_shift(Key::S));
    assert_eq!(
        bindings.get(KeybindAction::RectFillTool),
        KeyBinding::with_shift(Key::Q)
    );
    assert_eq!(bindings.get(KeybindAction::Undo), KeyBinding::with_ctrl(Key::Z));
    assert_eq!(bindings.get(KeybindAction::Redo), KeyBinding::with_ctrl(Key::Y));
    assert_eq!(bindings.get(KeybindAction::Save), KeyBinding::with_ctrl(Key::S));
    assert_eq!(
        bindings.get(KeybindAction::ShowPixelGrid),
        KeyBinding::with_shift(Key::G)
    );

    let recent = [
        Key::Key1,
        Key::Key2,
        Key::Key3,
        Key::Key4,
        Key::Key5,
        Key::Key6,
        Key::Key7,
        Key::Key8,
        Key::Key9,
        Key::Key0,
    ];
    for (index, key) in recent.into_iter().enumerate() {
        assert_eq!(bindings.recent(index), Some(KeyBinding::new(key)));
    }
    assert_eq!(bindings.recent(recent.len()), None);
}

#[test]
fn strong_dmm_keybindings_match_the_selected_shortcuts() {
    let bindings = KeybindPreset::StrongDmm.bindings();
    let expected = [
        (KeybindAction::Save, KeyBinding::with_ctrl(Key::S)),
        (KeybindAction::Undo, KeyBinding::with_ctrl(Key::Z)),
        (KeybindAction::Redo, KeyBinding::with_ctrl_shift(Key::Z)),
        (KeybindAction::ToggleAreaLayer, KeyBinding::with_ctrl(Key::Key1)),
        (KeybindAction::ToggleMobLayer, KeyBinding::with_ctrl(Key::Key4)),
        (KeybindAction::ShowAreas, KeyBinding::new(Key::A)),
        (KeybindAction::ShowAreaOutlines, KeyBinding::with_shift(Key::O)),
        (KeybindAction::LevelUp, KeyBinding::with_ctrl(Key::UpArrow)),
        (KeybindAction::LevelDown, KeyBinding::with_ctrl(Key::DownArrow)),
        (KeybindAction::Refit, KeyBinding::new(Key::Home)),
        (KeybindAction::PlaceTool, KeyBinding::new(Key::Key1)),
        (KeybindAction::PickTool, KeyBinding::new(Key::S)),
        (KeybindAction::MoveTool, KeyBinding::new(Key::M)),
        (KeybindAction::NodeTool, KeyBinding::new(Key::N)),
        (KeybindAction::SelectTool, KeyBinding::new(Key::Key3)),
        (KeybindAction::DeleteTool, KeyBinding::new(Key::D)),
        (KeybindAction::ReplaceTool, KeyBinding::new(Key::Key4)),
        (KeybindAction::FillTool, KeyBinding::with_shift(Key::Key2)),
        (KeybindAction::RectFillTool, KeyBinding::new(Key::Key2)),
        (KeybindAction::Rotate, KeyBinding::with_shift(Key::R)),
        (KeybindAction::Copy, KeyBinding::with_ctrl(Key::C)),
        (KeybindAction::Cut, KeyBinding::with_ctrl(Key::X)),
        (KeybindAction::Delete, KeyBinding::new(Key::Delete)),
        (KeybindAction::Paste, KeyBinding::with_ctrl(Key::V)),
        (KeybindAction::Find, KeyBinding::with_ctrl(Key::F)),
        (KeybindAction::FindNext, KeyBinding::new(Key::F3)),
        (KeybindAction::GoTo, KeyBinding::with_ctrl(Key::G)),
        (KeybindAction::ShowTileGrid, KeyBinding::new(Key::G)),
        (KeybindAction::ShowPixelGrid, KeyBinding::with_shift(Key::G)),
    ];

    for (action, binding) in expected {
        assert_eq!(bindings.get(action), binding);
    }

    for (index, key) in [
        Key::Q,
        Key::W,
        Key::E,
        Key::R,
        Key::T,
        Key::Y,
        Key::U,
        Key::I,
        Key::O,
        Key::P,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(bindings.recent(index), Some(KeyBinding::new(key)));
    }
}

#[test]
fn a_new_default_that_clashes_with_a_saved_binding_moves_the_later_action() {
    let mut saved = KeybindPreset::StrongDmm.bindings();
    saved.show_areas = KeyBinding::with_ctrl(Key::Key1);
    saved.toggle_area_layer = KeyBindings::default().toggle_area_layer;
    saved.resolve_duplicates();

    assert_eq!(saved.toggle_area_layer, KeyBindings::default().toggle_area_layer);
    for (index, action) in KeybindAction::ALL.into_iter().enumerate() {
        for other in KeybindAction::ALL.into_iter().skip(index + 1) {
            assert_ne!(saved.get(action), saved.get(other), "{action:?} and {other:?}");
        }
    }
}

#[test]
fn every_preset_uses_unique_bindable_keys() {
    for preset in [KeybindPreset::Default, KeybindPreset::StrongDmm] {
        let bindings = preset.bindings();

        for (index, action) in KeybindAction::ALL.into_iter().enumerate() {
            let binding = bindings.get(action);
            assert!(
                BINDABLE_KEYS.contains(&binding.key()) || (action.is_held() && MODIFIER_KEYS.contains(&binding.key())),
                "{preset:?} binding for {action:?} uses non-bindable key {:?}",
                binding.key()
            );
            for candidate in KeybindAction::ALL.into_iter().skip(index + 1) {
                assert_ne!(
                    binding,
                    bindings.get(candidate),
                    "{preset:?} assigns the same binding to {action:?} and {candidate:?}"
                );
            }
        }
    }
}

#[test]
fn missing_fields_use_their_defaults() {
    let settings: Settings = toml::from_str("show_areas = true\n").unwrap();

    assert_eq!(
        settings,
        Settings {
            show_areas: true,
            ..Settings::default()
        }
    );
}

#[test]
fn trusting_a_codebase_is_remembered_by_absolute_path() {
    let mut settings = Settings::default();
    assert!(!settings.is_trusted(Path::new("station.dme")));

    settings.trust_codebase(Path::new("station.dme"));
    settings.trust_codebase(Path::new("station.dme"));

    assert!(settings.is_trusted(Path::new("station.dme")));
    assert!(settings.is_trusted(&absolute(Path::new("station.dme"))));
    assert!(!settings.is_trusted(Path::new("other.dme")));
    assert_eq!(settings.trusted_codebases.len(), 1);
}

#[test]
fn settings_written_before_trusted_codebases_existed_still_load() {
    let settings: Settings = toml::from_str("recent_codebases = []\n").unwrap();

    assert!(settings.trusted_codebases.is_empty());
}

#[test]
fn older_keybinding_tables_receive_new_action_defaults() {
    let mut stored = toml::Value::try_from(KeyBindings::default()).unwrap();
    let table = stored.as_table_mut().unwrap();
    table.remove("save");
    table.remove("undo");
    table.remove("redo");
    for field in [
        "recent_1", "recent_2", "recent_3", "recent_4", "recent_5", "recent_6", "recent_7", "recent_8", "recent_9",
        "recent_0",
    ] {
        table.remove(field);
    }

    let bindings: KeyBindings = stored.try_into().unwrap();

    assert_eq!(bindings.get(KeybindAction::Save), KeyBinding::with_ctrl(Key::S));
    assert_eq!(bindings.get(KeybindAction::Undo), KeyBinding::with_ctrl(Key::Z));
    assert_eq!(bindings.get(KeybindAction::Redo), KeyBinding::with_ctrl(Key::Y));
    assert_eq!(bindings.recent(0), Some(KeyBinding::new(Key::Key1)));
    assert_eq!(bindings.recent(9), Some(KeyBinding::new(Key::Key0)));
}

#[test]
fn renamed_tool_bindings_keep_their_stored_keys() {
    let mut stored = toml::Value::try_from(KeyBindings::default()).unwrap();
    let table = stored.as_table_mut().unwrap();
    assert!(!table.contains_key("pick_tool"));
    table.insert(
        String::from("select_tool"),
        toml::Value::try_from(KeyBinding::with_ctrl(Key::P)).unwrap(),
    );
    table.insert(
        String::from("block_select_tool"),
        toml::Value::try_from(KeyBinding::with_ctrl(Key::B)).unwrap(),
    );

    let bindings: KeyBindings = stored.try_into().unwrap();

    assert_eq!(bindings.get(KeybindAction::PickTool), KeyBinding::with_ctrl(Key::P));
    assert_eq!(bindings.get(KeybindAction::SelectTool), KeyBinding::with_ctrl(Key::B));
}

#[test]
fn settings_written_before_the_option_existed_keep_the_marching_outline() {
    let settings: Settings = toml::from_str("tile_place_flash = false\n").unwrap();

    assert_eq!(settings.selection_highlight, SelectionHighlight::Outline);
    assert_eq!(settings.preferred_editor, DEFAULT_PREFERRED_EDITOR);
    assert_eq!(settings.selection_highlight.style(), HighlightStyle::Outline);
    assert_eq!(SelectionHighlight::Tint.style(), HighlightStyle::Tint);
    assert!(settings.object_tree_line_indicators);
    assert_eq!(settings.object_tree_search, ObjectTreeSearchOptions::default());
    assert_eq!(settings.object_tree_filter, ObjectTreeFilterOptions::default());
    assert!(settings.optimizations_enabled);
    assert!(settings.check_for_updates);
    assert_eq!(settings.minimum_light_brightness_percent, 0);
}

#[test]
fn object_tree_search_settings_keep_one_mode_enabled() {
    let mut settings: Settings = toml::from_str("[object_tree_search]\ntype_paths = false\nnames = false\n").unwrap();

    settings.normalize();

    assert_eq!(settings.object_tree_search, ObjectTreeSearchOptions::default());
}

#[test]
fn minimum_light_brightness_is_clamped_when_settings_load() {
    let stored = toml::from_str("minimum_light_brightness_percent = 250\n").unwrap();
    let loaded = SettingsLoad::from_file_result(Ok(Some(stored)));

    assert_eq!(loaded.settings.minimum_light_brightness_percent, 100);
}

#[test]
fn a_stored_font_section_loads_every_key() {
    let stored = toml::from_str(
        "[font]\npath = \"/fonts/Caskaydia.ttf\"\nsize = 20\nbrightness_percent = 120\npixel_snap = true\n",
    )
    .unwrap();
    let loaded = SettingsLoad::from_file_result(Ok(Some(stored)));

    assert_eq!(
        loaded.settings.font,
        FontSettings {
            path: Some(PathBuf::from("/fonts/Caskaydia.ttf")),
            size: 20,
            brightness_percent: 120,
            pixel_snap: true,
        }
    );
}

#[test]
fn ui_scale_defaults_to_the_os_and_stays_in_the_supported_range() {
    let missing: Settings = toml::from_str("").unwrap();
    assert_eq!(missing.ui_scale_override_percent, None);
    let previous_default: Settings = toml::from_str("ui_scale_percent = 100\n").unwrap();
    assert_eq!(previous_default.ui_scale_override_percent, None);

    for (stored, expected) in [(1, 50), (250, 200)] {
        let settings = toml::from_str(&format!("ui_scale_override_percent = {stored}\n")).unwrap();
        let loaded = SettingsLoad::from_file_result(Ok(Some(settings)));
        assert_eq!(loaded.settings.ui_scale_override_percent, Some(expected));
    }
}

#[test]
fn settings_round_trip_through_toml() {
    let mut settings = Settings {
        maximized: true,
        preferred_editor: String::from("zed {file}:{line}:{column}"),
        show_areas: true,
        show_area_outlines: false,
        show_lighting: true,
        minimum_light_brightness_percent: 35,
        focus_windows_on_hover: false,
        ui_scale_override_percent: Some(150),
        theme: String::from("custom"),
        font: FontSettings::default(),
        tile_place_flash: false,
        selection_guide_line: false,
        show_tile_grid: false,
        tile_grid_min_pixels: 12,
        show_tile_grid_axis: false,
        show_selected_pixel_grid: false,
        mirror_camera: true,
        selected_pixel_grid_min_pixels: 5,
        show_pixel_grid_axis: false,
        selection_highlight: SelectionHighlight::Tint,
        object_tree_line_indicators: false,
        object_tree_search: ObjectTreeSearchOptions {
            type_paths: false,
            names: true,
        },
        object_tree_filter: ObjectTreeFilterOptions {
            atom: true,
            movable: false,
            obj: true,
            turf: false,
            custom_enabled: true,
            custom_type_path: String::from("/atom/movable/lighting"),
        },
        keybindings: KeyBindings::default(),
        recent_codebases: Vec::new(),
        trusted_codebases: Vec::new(),
        recent: Vec::new(),
        profile_selections: vec![ProfileSelection {
            environment: PathBuf::from("/project/station.dme"),
            profile: String::from("/datum/demir/tgstation/debug"),
        }],
        forced_profile_selections: vec![ForcedProfileSelection {
            environment: PathBuf::from("/project/colonialmarines.dme"),
            profile: String::from("cmss13"),
        }],
        optimizations_enabled: false,
        bake_enabled: true,
        git_enabled: true,
        blame_depth: 500,
        check_for_updates: false,
        sanitize_vars_on_save: true,
        pinned_vars: vec![String::from("req_access")],
        pinned_types: vec![String::from("/obj/machinery")],
        coop: CoopSettings::default(),
        panels: OpenPanels::default(),
        show_dm_ui: true,
    };
    settings
        .keybindings
        .rebind(KeybindAction::ShowAreas, KeyBinding::with_ctrl(Key::G));
    settings
        .keybindings
        .rebind(KeybindAction::Recent1, KeyBinding::with_shift(Key::F2));
    let encoded = toml::to_string_pretty(&settings).unwrap();

    assert!(encoded.contains("[keybindings.show_areas]"));
    assert!(encoded.contains("[keybindings.recent_1]"));
    assert!(encoded.contains("maximized = true"));
    assert!(encoded.contains("preferred_editor = \"zed {file}:{line}:{column}\""));
    assert!(encoded.contains("optimizations_enabled = false"));
    assert!(encoded.contains("check_for_updates = false"));
    assert!(encoded.contains("selection_highlight = \"tint\""));
    assert!(encoded.contains("object_tree_line_indicators = false"));
    assert!(encoded.contains("[object_tree_search]"));
    assert!(encoded.contains("type_paths = false"));
    assert!(encoded.contains("names = true"));
    assert!(encoded.contains("[object_tree_filter]"));
    assert!(encoded.contains("atom = true"));
    assert!(encoded.contains("obj = true"));
    assert!(encoded.contains("custom_enabled = true"));
    assert!(encoded.contains("custom_type_path = \"/atom/movable/lighting\""));
    assert!(encoded.contains("minimum_light_brightness_percent = 35"));
    assert!(encoded.contains("ui_scale_override_percent = 150"));
    assert!(encoded.contains("[[profile_selections]]"));
    assert!(encoded.contains("profile = \"/datum/demir/tgstation/debug\""));
    assert!(encoded.contains("[[forced_profile_selections]]"));
    assert!(encoded.contains("profile = \"cmss13\""));
    assert!(encoded.contains("key = \"G\""));
    assert!(encoded.contains("ctrl = true"));
    assert_eq!(toml::from_str::<Settings>(&encoded).unwrap(), settings);
}

#[test]
fn frame_options_apply_and_capture_without_touching_other_settings() {
    let mut settings = Settings {
        maximized: true,
        preferred_editor: String::from("editor {file}"),
        show_areas: true,
        show_area_outlines: false,
        show_lighting: true,
        minimum_light_brightness_percent: 35,
        focus_windows_on_hover: true,
        ui_scale_override_percent: None,
        theme: String::from("custom"),
        font: FontSettings::default(),
        tile_place_flash: false,
        selection_guide_line: false,
        show_tile_grid: true,
        tile_grid_min_pixels: 10,
        show_tile_grid_axis: true,
        show_selected_pixel_grid: true,
        mirror_camera: false,
        selected_pixel_grid_min_pixels: 3,
        show_pixel_grid_axis: true,
        selection_highlight: SelectionHighlight::Tint,
        object_tree_line_indicators: false,
        object_tree_search: ObjectTreeSearchOptions {
            type_paths: false,
            names: true,
        },
        object_tree_filter: ObjectTreeFilterOptions::default(),
        keybindings: KeyBindings::default(),
        recent_codebases: Vec::new(),
        trusted_codebases: Vec::new(),
        recent: Vec::new(),
        profile_selections: Vec::new(),
        forced_profile_selections: Vec::new(),
        optimizations_enabled: true,
        bake_enabled: true,
        git_enabled: true,
        blame_depth: 500,
        check_for_updates: true,
        sanitize_vars_on_save: false,
        pinned_vars: Vec::new(),
        pinned_types: Vec::new(),
        coop: CoopSettings::default(),
        panels: OpenPanels::default(),
        show_dm_ui: true,
    };
    let mut options = FrameOptions::default();

    settings.apply_to(&mut options);
    assert!(options.show_areas);
    assert!(!options.show_area_outlines);
    assert_eq!(options.minimum_light_brightness_percent, 35);

    options.show_areas = false;
    options.show_area_outlines = true;
    options.minimum_light_brightness_percent = 60;
    settings.capture_from(&options);
    assert_eq!(
        settings,
        Settings {
            maximized: true,
            preferred_editor: String::from("editor {file}"),
            theme: String::from("custom"),
            tile_place_flash: false,
            selection_guide_line: false,
            selection_highlight: SelectionHighlight::Tint,
            object_tree_line_indicators: false,
            minimum_light_brightness_percent: 60,
            object_tree_search: ObjectTreeSearchOptions {
                type_paths: false,
                names: true,
            },
            ..Settings::default()
        }
    );
}

#[test]
fn malformed_toml_is_rejected() {
    assert!(toml::from_str::<Settings>("show_areas = maybe").is_err());
}

#[test]
fn only_a_missing_settings_file_requests_a_keybinding_preset() {
    let missing = Settings::from_file_read(Err(io::Error::from(io::ErrorKind::NotFound))).unwrap();
    let missing = SettingsLoad::from_file_result(Ok(missing));
    assert!(missing.needs_keybind_preset);

    let present = Settings::from_file_read(Ok(String::new())).unwrap();
    let present = SettingsLoad::from_file_result(Ok(present));
    assert!(!present.needs_keybind_preset);
    assert_eq!(present.settings, Settings::default());

    let malformed = Settings::from_file_read(Ok(String::from("show_areas = maybe")));
    let malformed = SettingsLoad::from_file_result(malformed);
    assert!(!malformed.needs_keybind_preset);

    let unreadable = Settings::from_file_read(Err(io::Error::from(io::ErrorKind::PermissionDenied)));
    let unreadable = SettingsLoad::from_file_result(unreadable);
    assert!(!unreadable.needs_keybind_preset);
}

#[test]
fn rebinding_to_an_assigned_chord_swaps_the_actions() {
    let mut bindings = KeyBindings::default();

    bindings.rebind(KeybindAction::ShowAreas, KeyBinding::new(Key::W));

    assert_eq!(bindings.get(KeybindAction::ShowAreas), KeyBinding::new(Key::W));
    assert_eq!(bindings.get(KeybindAction::PlaceTool), KeyBinding::new(Key::A));
}

#[test]
fn recent_maps_move_to_the_front_without_duplicating() {
    let mut settings = Settings::default();
    let env = Path::new("station.dme");

    settings.push_recent(Some(env), Path::new("a.dmm"));
    settings.push_recent(Some(env), Path::new("b.dmm"));
    settings.push_recent(Some(env), Path::new("a.dmm"));

    assert_eq!(
        settings.recent,
        vec![
            RecentMap {
                map: absolute(Path::new("a.dmm")),
                environment: Some(absolute(env)),
            },
            RecentMap {
                map: absolute(Path::new("b.dmm")),
                environment: Some(absolute(env)),
            },
        ]
    );
}

#[test]
fn recent_entries_are_stored_absolute_so_they_survive_a_different_working_directory() {
    let mut settings = Settings::default();

    settings.push_recent(Some(Path::new("env/test.dme")), Path::new("env/test.dmm"));

    assert!(settings.recent[0].map.is_absolute());
    assert!(
        settings.recent[0]
            .environment
            .as_ref()
            .is_some_and(|path| path.is_absolute())
    );
}

#[test]
fn the_same_map_named_two_ways_is_one_recent_entry() {
    let mut settings = Settings::default();
    let relative = Path::new("maps/station.dmm");

    settings.push_recent(None, relative);
    settings.push_recent(None, &absolute(relative));

    assert_eq!(settings.recent.len(), 1);
}

#[test]
fn recent_maps_remember_a_map_opened_without_an_environment() {
    let mut settings = Settings::default();

    settings.push_recent(None, Path::new("lone.dmm"));

    assert_eq!(settings.recent[0].environment, None);
}

#[test]
fn the_recent_list_is_capped() {
    let mut settings = Settings::default();

    for index in 0..MAX_RECENT + 5 {
        settings.push_recent(None, Path::new(&format!("map{index}.dmm")));
    }

    assert_eq!(settings.recent.len(), MAX_RECENT);
    assert_eq!(settings.recent[0].map, absolute(Path::new("map14.dmm")));
}

#[test]
fn recent_maps_round_trip_through_toml() {
    let mut settings = Settings::default();
    settings.push_recent(Some(Path::new("station.dme")), Path::new("station.dmm"));
    let encoded = toml::to_string_pretty(&settings).unwrap();

    assert_eq!(toml::from_str::<Settings>(&encoded).unwrap(), settings);
}

#[test]
fn recent_codebases_move_to_the_front_without_duplicating() {
    let mut settings = Settings::default();

    settings.push_codebase(Path::new("a.dme"));
    settings.push_codebase(Path::new("b.dme"));
    settings.push_codebase(Path::new("a.dme"));

    assert_eq!(
        settings.recent_codebases,
        vec![absolute(Path::new("a.dme")), absolute(Path::new("b.dme"))]
    );
}

#[test]
fn profile_overrides_are_scoped_replaced_and_cleared_per_codebase() {
    let mut settings = Settings::default();
    let station = Path::new("station.dme");
    let other = Path::new("other.dme");

    settings.set_profile_for(station, Some("/datum/demir/tgstation/debug"));
    settings.set_profile_for(other, Some("/datum/demir/goonstation"));
    assert_eq!(settings.profile_for(station), Some("/datum/demir/tgstation/debug"));
    assert_eq!(settings.profile_for(other), Some("/datum/demir/goonstation"));

    settings.set_profile_for(station, Some("/datum/demir/tgstation/lighting"));
    assert_eq!(settings.profile_for(station), Some("/datum/demir/tgstation/lighting"));
    assert_eq!(settings.profile_selections.len(), 2);

    settings.set_profile_for(station, None);
    assert_eq!(settings.profile_for(station), None);
    assert_eq!(settings.profile_for(other), Some("/datum/demir/goonstation"));
}

#[test]
fn forced_profiles_are_scoped_replaced_and_cleared_per_codebase() {
    let mut settings = Settings::default();
    let station = Path::new("station.dme");
    let other = Path::new("other.dme");

    settings.set_forced_profile_for(station, Some("tgstation"));
    settings.set_forced_profile_for(other, Some("goonstation"));
    assert_eq!(settings.forced_profile_for(station), Some("tgstation"));
    assert_eq!(settings.forced_profile_for(other), Some("goonstation"));

    settings.set_forced_profile_for(station, Some("vanderlin"));
    assert_eq!(settings.forced_profile_for(station), Some("vanderlin"));
    assert_eq!(settings.forced_profile_selections.len(), 2);

    settings.set_forced_profile_for(station, None);
    assert_eq!(settings.forced_profile_for(station), None);
    assert_eq!(settings.forced_profile_for(other), Some("goonstation"));
}

#[test]
fn forced_profiles_stored_as_the_old_enum_map_to_file_names() {
    let stored = toml::from_str(
        "[[forced_profile_selections]]\nenvironment = \"/project/city.dme\"\nprofile = \"second_city\"\n",
    )
    .unwrap();
    let loaded = SettingsLoad::from_file_result(Ok(Some(stored)));

    assert_eq!(loaded.settings.forced_profile_selections[0].profile, "secondcity");
}

#[test]
fn recent_maps_are_filtered_to_one_codebase() {
    let mut settings = Settings::default();
    let station = Path::new("station.dme");

    settings.push_recent(Some(station), Path::new("one.dmm"));
    settings.push_recent(Some(Path::new("other.dme")), Path::new("two.dmm"));
    settings.push_recent(Some(station), Path::new("three.dmm"));

    let maps = settings
        .recent_maps_for(station)
        .map(|recent| &recent.map)
        .collect::<Vec<_>>();

    assert_eq!(
        maps,
        [&absolute(Path::new("three.dmm")), &absolute(Path::new("one.dmm"))]
    );
}

#[test]
fn a_map_opened_without_a_codebase_belongs_to_no_codebase() {
    let mut settings = Settings::default();

    settings.push_recent(None, Path::new("lone.dmm"));

    assert_eq!(settings.recent_maps_for(Path::new("station.dme")).count(), 0);
}

#[test]
fn platform_paths_use_the_requested_application_directory() {
    let root = Path::new("profile");

    assert_eq!(settings_path_from(root, true), root.join("rmd/settings.toml"));
    assert_eq!(settings_path_from(root, false), root.join(".config/rmd/settings.toml"));
    assert_eq!(
        settings_path_from(root, true).with_file_name("imgui.ini"),
        root.join("rmd/imgui.ini")
    );
    assert_eq!(
        settings_path_from(root, false).with_file_name("imgui.ini"),
        root.join(".config/rmd/imgui.ini")
    );
    assert_eq!(
        settings_path_from(root, true).with_file_name("latest.log"),
        root.join("rmd/latest.log")
    );
    assert_eq!(
        settings_path_from(root, false).with_file_name("latest.log"),
        root.join(".config/rmd/latest.log")
    );
}
