use dear_imgui_rs::Context;

use super::{style::STYLE_COLORS, *};

fn default_style() -> Style {
    Context::create().style().clone()
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rmd-theme-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    dir
}

#[test]
fn style_colors_cover_every_color_once() {
    for (index, (color, _)) in STYLE_COLORS.iter().enumerate() {
        assert_eq!(*color as usize, index);
    }
}

#[test]
fn only_web_urls_can_be_opened() {
    let theme = |url: &str| Theme {
        name: String::new(),
        url: url.to_owned(),
        style: default_style(),
    };
    let _context = crate::testing::CONTEXT_LOCK.lock().unwrap();

    assert_eq!(
        theme(" https://github.com/a/b ").web_url(),
        Some("https://github.com/a/b")
    );
    assert_eq!(theme("HTTP://example.com").web_url(), Some("HTTP://example.com"));
    assert_eq!(theme("file:///tmp/payload").web_url(), None);
    assert_eq!(theme("\\\\server\\share").web_url(), None);
    assert_eq!(theme("").web_url(), None);
}

#[test]
fn builtin_files_fill_missing_keys_from_their_own_preset() {
    let _context = crate::testing::CONTEXT_LOCK.lock().unwrap();
    let dir = scratch_dir("preset-base");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("imgui_light.toml"), "name = \"ImGui Light\"\n").unwrap();

    let themes = Themes::load(Some(dir.clone()), &default_style(), LIGHT_THEME);
    assert!(themes.is_active_default());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hex_colors_round_trip() {
    assert_eq!(hex_color([1.0, 0.0, 0.5, 0.94]), "#FF0080F0");
    assert_eq!(
        parse_hex_color("#FF0080F0").map(hex_color).as_deref(),
        Some("#FF0080F0")
    );
    assert_eq!(parse_hex_color("FF0080"), Some([1.0, 0.0, 128.0 / 255.0, 1.0]));
    assert_eq!(parse_hex_color("#FF00"), None);
    assert_eq!(parse_hex_color("#GG0000FF"), None);
}

#[test]
fn builtin_themes_survive_a_round_trip() {
    let _context = crate::testing::CONTEXT_LOCK.lock().unwrap();
    let default_style = default_style();
    for (id, theme) in builtin_themes(&default_style) {
        let written = theme.to_toml().unwrap();
        let read = Theme::parse(id, &written, &default_style).unwrap();
        assert_eq!(read, theme);
    }
}

#[test]
fn partial_theme_keeps_defaults_and_ignores_unknown_keys() {
    let _context = crate::testing::CONTEXT_LOCK.lock().unwrap();
    let base = default_style();
    let source = r##"
        [style]
        WindowRounding = 6
        Alpha = 4.0
        Bogus = 1.0

        [colors]
        WindowBg = "#102030FF"
        NotAColor = "#000000FF"
    "##;
    let theme = Theme::parse("partial", source, &base).unwrap();

    assert_eq!(theme.name, "partial");
    assert_eq!(theme.style.window_rounding(), 6.0);
    assert_eq!(theme.style.alpha(), 1.0);
    assert_eq!(
        theme.style.color(StyleColor::WindowBg),
        parse_hex_color("#102030FF").unwrap()
    );
    assert_eq!(theme.style.window_padding(), base.window_padding());
    assert_eq!(theme.style.color(StyleColor::Text), base.color(StyleColor::Text));
}

#[test]
fn loading_writes_builtins_and_falls_back_to_the_default() {
    let _context = crate::testing::CONTEXT_LOCK.lock().unwrap();
    let dir = scratch_dir("load");
    let default_style = default_style();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("custom.toml"),
        "name = \"Custom\"\n[colors]\nText = \"#FF0000FF\"\n",
    )
    .unwrap();
    fs::write(dir.join("broken.toml"), "name = ").unwrap();

    let themes = Themes::load(Some(dir.clone()), &default_style, "missing");
    assert!(dir.join("imgui_dark.toml").exists());
    assert!(dir.join("imgui_light.toml").exists());
    assert_eq!(themes.active_id(), DEFAULT_THEME);
    assert_eq!(
        themes.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        ["custom", "imgui_dark", "imgui_light"]
    );

    let mut themes = Themes::load(Some(dir.clone()), &default_style, "custom");
    assert_eq!(themes.active_theme().name, "Custom");
    themes.active_theme_mut().style.set_window_rounding(9.0);
    assert!(themes.is_dirty());
    themes.revert_active();
    assert!(!themes.is_dirty());

    themes.active_theme_mut().style.set_window_rounding(9.0);
    themes.save_active().unwrap();
    assert!(!themes.is_dirty());
    let themes = Themes::load(Some(dir.clone()), &default_style, "custom");
    assert_eq!(themes.active_style().window_rounding(), 9.0);

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn builtins_reset_and_new_themes_copy_the_active_one() {
    let _context = crate::testing::CONTEXT_LOCK.lock().unwrap();
    let dir = scratch_dir("create");
    let mut themes = Themes::load(Some(dir.clone()), &default_style(), DEFAULT_THEME);
    themes.active_theme_mut().style.set_window_rounding(7.0);
    themes.save_active().unwrap();

    let mut themes = Themes::load(Some(dir.clone()), &default_style(), DEFAULT_THEME);
    assert!(themes.is_active_builtin());
    assert!(themes.select(LIGHT_THEME));
    assert!(themes.is_active_default());
    assert!(themes.select(DEFAULT_THEME));
    assert!(!themes.is_active_default());
    themes.reset_active();
    assert!(themes.is_active_default());
    assert!(themes.is_dirty());

    themes.active_theme_mut().style.set_window_rounding(3.0);
    assert_eq!(themes.create("  My Theme! ").unwrap(), "my_theme");
    assert_eq!(themes.create("My Theme").unwrap(), "my_theme_2");
    assert_eq!(themes.create("???").unwrap(), "theme");
    assert_eq!(themes.active_theme().name, "???");
    assert!(!themes.is_active_builtin());
    assert!(!themes.is_dirty());

    let themes = Themes::load(Some(dir.clone()), &default_style(), "my_theme");
    assert_eq!(themes.active_theme().name, "My Theme!");
    assert_eq!(themes.active_style().window_rounding(), 3.0);

    fs::remove_dir_all(dir).unwrap();
}
