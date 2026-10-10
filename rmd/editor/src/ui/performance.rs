use std::fmt::Write;

use dear_imgui_rs::{TableFlags, TableSizingPolicy, TreeNodeFlags, Ui, WindowKey, WindowKeyError};
use render::{
    device::HeapUsage,
    stats::{BufferPlacement, RenderSpan, RendererInfo},
};

use crate::{
    profiler::{EditorSpan, FrameHistory, Summary, UploadCounts, millis},
    session::Session,
};

const MIB: f64 = 1024.0 * 1024.0;

pub(super) struct PerformancePanel {
    window: WindowKey,
}

pub(super) struct ReportContext {
    map_size: Option<[u32; 3]>,
    underlay_depth: u32,
    show_lighting: bool,
}

impl ReportContext {
    pub(super) fn new(session: &Session) -> Self {
        Self {
            map_size: session.state.active_document().map(|document| {
                let size = document.map.size();
                [size.x, size.y, size.z]
            }),
            underlay_depth: session.options.underlay_depth,
            show_lighting: session.options.show_lighting,
        }
    }
}

impl PerformancePanel {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("performance-panel", "Performance")?,
        })
    }

    /// Returns the report text once "Copy report" is pressed.
    pub(super) fn draw(
        &self, ui: &Ui, history: &FrameHistory, info: &RendererInfo, context: &ReportContext, open: &mut bool,
    ) -> Option<String> {
        let mut copy = None;
        ui.window(self.window.label("Performance")).opened(open).build(|| {
            if ui.button("Copy report") {
                copy = Some(report(history, info, context));
            }
            ui.same_line();
            ui.text_disabled(format!("last {} drawn frames", history.len()));

            for line in header(history, info, context) {
                ui.text_wrapped(line);
            }

            let frame = history.editor(EditorSpan::Frame);
            ui.plot_lines_config("##frame", &history.frame_millis())
                .overlay_text(format!("cpu frame, avg {:.2} ms", millis(frame.average)))
                .scale_min(0.0)
                .graph_size([-1.0, 60.0])
                .build();
            let gpu = history.gpu_total();
            ui.plot_lines_config("##gpu", &history.gpu_millis())
                .overlay_text(format!("gpu passes, avg {:.2} ms", millis(gpu.average)))
                .scale_min(0.0)
                .graph_size([-1.0, 60.0])
                .build();

            if ui.collapsing_header("Editor CPU", TreeNodeFlags::DEFAULT_OPEN) {
                let rows = EditorSpan::ALL.map(|span| (span.label().to_owned(), history.editor(span)));
                summary_table(ui, "editor-spans", &rows);
            }

            if ui.collapsing_header("Renderer CPU", TreeNodeFlags::DEFAULT_OPEN) {
                let rows = RenderSpan::ALL.map(|span| (span.label().to_owned(), history.renderer(span)));
                summary_table(ui, "renderer-spans", &rows);
            }

            if ui.collapsing_header("GPU passes", TreeNodeFlags::DEFAULT_OPEN) {
                if !info.has_timestamps {
                    ui.text_disabled("this GPU cannot time passes");
                }

                let mut rows = history
                    .gpu_passes()
                    .into_iter()
                    .map(|(name, summary)| (name.to_string(), summary))
                    .collect::<Vec<_>>();
                rows.push((String::from("sum of passes"), gpu));
                summary_table(ui, "gpu-passes", &rows);
            }

            if ui.collapsing_header("Memory", TreeNodeFlags::DEFAULT_OPEN) {
                for line in memory(info) {
                    ui.text(line);
                }
            }
        });

        copy
    }
}

fn summary_table(ui: &Ui, id: &str, rows: &[(String, Summary)]) {
    ui.table(id)
        .flags(TableFlags::ROW_BG | TableFlags::BORDERS_INNER_V | TableFlags::BORDERS_OUTER)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .headers(true)
        .column("Span")
        .weight(3.0)
        .done()
        .column("avg ms")
        .weight(1.0)
        .done()
        .column("p95 ms")
        .weight(1.0)
        .done()
        .column("max ms")
        .weight(1.0)
        .done()
        .column("frames")
        .weight(1.0)
        .done()
        .build(|ui| {
            for (name, summary) in rows {
                ui.table_next_row();
                ui.table_next_column();
                ui.text(name);
                for value in [summary.average, summary.p95, summary.max] {
                    ui.table_next_column();
                    ui.text(format!("{:.2}", millis(value)));
                }
                ui.table_next_column();
                ui.text(summary.count.to_string());
            }
        });
}

fn header(history: &FrameHistory, info: &RendererInfo, context: &ReportContext) -> Vec<String> {
    let version = info.device.api_version;
    let mut lines = vec![
        format!(
            "{} ({:?}), {}, Vulkan {}.{}.{}",
            info.device.name,
            info.device.device_type,
            info.device.driver,
            version >> 22 & 0x7f,
            version >> 12 & 0x3ff,
            version & 0xfff,
        ),
        format!(
            "{} swapchain images, gpu timestamps {}",
            info.swapchain_images,
            if info.has_timestamps { "on" } else { "unsupported" }
        ),
    ];

    let map = context
        .map_size
        .map_or_else(|| String::from("no map"), |[x, y, z]| format!("map {x}x{y}x{z}"));
    let views = history
        .views()
        .iter()
        .map(|view| format!("{} underlay + {} active", view.underlay, view.active))
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!(
        "{map}, {} sprites, views: [{views}], underlay depth {}, lighting {}",
        history.sprite_count(),
        context.underlay_depth,
        if context.show_lighting { "on" } else { "off" },
    ));

    let interval = history.interval();
    let fps = match interval.average.as_secs_f32() {
        0.0 => 0.0,
        seconds => 1.0 / seconds,
    };
    lines.push(format!(
        "frame interval avg {:.2} ms ({fps:.0} fps), p95 {:.2} ms, max {:.2} ms; graph re-recorded in {} frames",
        millis(interval.average),
        millis(interval.p95),
        millis(interval.max),
        history.records(),
    ));
    lines.push(format!(
        "sprite uploads: {}; lighting uploads: {}",
        uploads(history.sprite_uploads()),
        uploads(history.lighting_uploads()),
    ));

    lines
}

fn uploads(counts: UploadCounts) -> String {
    format!(
        "{} full ({:.1} MiB), {} incremental ({:.1} MiB)",
        counts.full,
        counts.full_bytes as f64 / MIB,
        counts.incremental,
        counts.incremental_bytes as f64 / MIB,
    )
}

fn memory(info: &RendererInfo) -> Vec<String> {
    let placement = |name: &str, buffer: Option<BufferPlacement>| match buffer {
        Some(buffer) => format!(
            "{name}: {:.1} MiB in {}",
            buffer.bytes as f64 / MIB,
            buffer.placement.map_or("unknown memory", |placement| placement.label())
        ),
        None => format!("{name}: not allocated"),
    };

    let mut lines = vec![
        format!(
            "textures: {} sheets, {:.1} MiB",
            info.texture_count,
            info.texture_bytes as f64 / MIB
        ),
        placement("sprite buffer", info.sprites),
        placement("light buffer", info.lights),
    ];
    lines.extend(
        info.heaps
            .iter()
            .enumerate()
            .map(|(index, heap)| heap_line(index, heap)),
    );

    lines
}

fn heap_line(index: usize, heap: &HeapUsage) -> String {
    let kind = match (heap.is_device_local, heap.is_host_visible) {
        (true, true) => "VRAM, host-visible",
        (true, false) => "VRAM",
        (false, _) => "system RAM",
    };
    let usage = match (heap.usage, heap.budget) {
        (Some(usage), Some(budget)) => {
            format!(
                ", used {:.1} of {:.1} MiB budget",
                usage as f64 / MIB,
                budget as f64 / MIB
            )
        },
        _ => String::new(),
    };

    format!("heap {index}: {:.1} MiB {kind}{usage}", heap.size as f64 / MIB)
}

fn report(history: &FrameHistory, info: &RendererInfo, context: &ReportContext) -> String {
    let mut text = String::from("rmd performance report\n");
    for line in header(history, info, context) {
        let _ = writeln!(text, "{line}");
    }

    let mut section = |title: &str, rows: Vec<(String, Summary)>| {
        let _ = writeln!(text, "\n{title} (avg / p95 / max ms, frames)");
        for (name, summary) in rows {
            let _ = writeln!(
                text,
                "  {name}: {:.2} / {:.2} / {:.2}, {}",
                millis(summary.average),
                millis(summary.p95),
                millis(summary.max),
                summary.count
            );
        }
    };
    section(
        "editor cpu",
        EditorSpan::ALL
            .map(|span| (span.label().to_owned(), history.editor(span)))
            .into(),
    );
    section(
        "renderer cpu",
        RenderSpan::ALL
            .map(|span| (span.label().to_owned(), history.renderer(span)))
            .into(),
    );
    let mut passes = history
        .gpu_passes()
        .into_iter()
        .map(|(name, summary)| (name.to_string(), summary))
        .collect::<Vec<_>>();
    passes.push((String::from("sum of passes"), history.gpu_total()));
    section("gpu passes", passes);

    let _ = writeln!(text, "\nmemory");
    for line in memory(info) {
        let _ = writeln!(text, "  {line}");
    }

    text
}
