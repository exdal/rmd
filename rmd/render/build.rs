use std::{env, error::Error, ffi::CString, fs, path::PathBuf};

use shader_slang as slang;
use slang::Downcast;

const MODULES: [(&str, &[(&str, &str)]); 7] = [
    (
        "sprite.slang",
        &[
            ("vs_main", "sprite.vert.spv"),
            ("fs_main", "sprite.frag.spv"),
            ("fs_visibility", "sprite_visibility.frag.spv"),
        ],
    ),
    (
        "sprite_cull.slang",
        &[
            ("cs_classify", "sprite_cull_classify.comp.spv"),
            ("cs_scan", "sprite_cull_scan.comp.spv"),
            ("cs_compact", "sprite_cull_compact.comp.spv"),
        ],
    ),
    (
        "blur.slang",
        &[("vs_main", "blur.vert.spv"), ("fs_main", "blur.frag.spv")],
    ),
    (
        "lighting.slang",
        &[("vs_main", "lighting.vert.spv"), ("fs_main", "lighting.frag.spv")],
    ),
    ("area_color.slang", &[("cs_main", "area_color.comp.spv")]),
    (
        "interaction.slang",
        &[
            ("vs_main", "interaction.vert.spv"),
            ("fs_main", "interaction.frag.spv"),
            ("cs_pick", "pick.comp.spv"),
        ],
    ),
    (
        "imgui.slang",
        &[("vs_main", "imgui.vert.spv"), ("fs_main", "imgui.frag.spv")],
    ),
];

const COMMON_MODULES: [&str; 6] = [
    "common.slang",
    "common/types.slang",
    "common/color.slang",
    "common/encoding.slang",
    "sprite_shared.slang",
    "spec.slang",
];

fn main() -> Result<(), Box<dyn Error>> {
    for (module, _) in MODULES {
        println!("cargo:rerun-if-changed=shaders/{module}");
    }
    for module in COMMON_MODULES {
        println!("cargo:rerun-if-changed=shaders/{module}");
    }
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let shader_dir = manifest_dir.join("shaders");

    let global_session = slang::GlobalSession::new().ok_or("failed to create Slang global session")?;

    let options = slang::CompilerOptions::default()
        .optimization(slang::OptimizationLevel::High)
        .matrix_layout_row(true)
        .glsl_force_scalar_layout(true);
    let targets = [slang::TargetDesc::default()
        .format(slang::CompileTarget::Spirv)
        .profile(global_session.find_profile("glsl_450"))
        .options(&options)];

    let search_path = CString::new(shader_dir.to_str().ok_or("non-UTF-8 shader path")?)?;
    let search_paths = [search_path.as_ptr()];

    let session_desc = slang::SessionDesc::default()
        .targets(&targets)
        .search_paths(&search_paths)
        .options(&options);
    let session = global_session
        .create_session(&session_desc)
        .ok_or("failed to create Slang session")?;

    for (module_name, entry_points) in MODULES {
        let module = session.load_module(module_name)?;

        let mut components = vec![module.downcast().clone()];
        for (entry_point, _) in entry_points {
            let entry_point = module
                .find_entry_point_by_name(entry_point)
                .ok_or_else(|| format!("entry point `{entry_point}` not found in {module_name}"))?;
            components.push(entry_point.downcast().clone());
        }

        let program = session.create_composite_component_type(&components)?;
        let linked = program.link()?;

        for (index, (_, file_name)) in entry_points.iter().enumerate() {
            let code = linked.entry_point_code(index as i64, 0)?;
            fs::write(out_dir.join(file_name), code.as_slice())?;
        }
    }

    Ok(())
}
