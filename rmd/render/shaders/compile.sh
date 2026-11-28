#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"
rm -rf spirv && mkdir spirv

compile() {
    slangc "$1" -target spirv -profile glsl_450 -O2 -matrix-layout-row-major -force-glsl-scalar-layout -I . -entry "$2" -o "spirv/$3"
}

compile sprite.slang vs_main sprite.vert.spv
compile sprite.slang fs_main sprite.frag.spv
compile sprite.slang fs_scene sprite_scene.frag.spv
compile sprite.slang fs_visibility sprite_visibility.frag.spv
compile sprite.slang fs_overlay_light sprite_overlay_light.frag.spv

compile sprite_cull.slang cs_classify sprite_cull_classify.comp.spv
compile sprite_cull.slang cs_scan sprite_cull_scan.comp.spv
compile sprite_cull.slang cs_compact sprite_cull_compact.comp.spv

compile blur.slang vs_main blur.vert.spv
compile blur.slang fs_main blur.frag.spv

compile lighting.slang vs_main lighting.vert.spv
compile lighting.slang fs_main lighting.frag.spv
compile lighting.slang fs_resolve scene_resolve.frag.spv

compile area_color.slang cs_main area_color.comp.spv

compile interaction.slang vs_main interaction.vert.spv
compile interaction.slang fs_main interaction.frag.spv
compile interaction.slang cs_pick pick.comp.spv

compile guide.slang vs_main guide.vert.spv
compile guide.slang fs_main guide.frag.spv

compile capture.slang cs_main capture.comp.spv

compile imgui.slang vs_main imgui.vert.spv
compile imgui.slang fs_main imgui.frag.spv
