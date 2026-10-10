use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use render::stats::{RenderSpan, RenderStats, Upload, ViewSprites};

const HISTORY: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSpan {
    PrepareFrame,
    Ui,
    MapViewFrames,
    ImGuiRender,
    Renderer,
    Polls,
    Frame,
}

impl EditorSpan {
    pub const ALL: [Self; 7] = [
        Self::PrepareFrame,
        Self::Ui,
        Self::MapViewFrames,
        Self::ImGuiRender,
        Self::Renderer,
        Self::Polls,
        Self::Frame,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::PrepareFrame => "platform prepare",
            Self::Ui => "ui",
            Self::MapViewFrames => "map view frames",
            Self::ImGuiRender => "imgui render",
            Self::Renderer => "renderer",
            Self::Polls => "background polls",
            Self::Frame => "whole frame",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct FrameSample {
    started: Option<Instant>,
    editor: [Duration; EditorSpan::ALL.len()],
    renderer: [Duration; RenderSpan::ALL.len()],
    gpu: Vec<(Arc<str>, Duration)>,
    sprite_upload: Upload,
    lighting_upload: Upload,
    is_recorded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    pub average: Duration,
    pub p95: Duration,
    pub max: Duration,
    /// samples the span or pass ran in
    pub count: usize,
}

impl Summary {
    fn of(mut durations: Vec<Duration>) -> Self {
        if durations.is_empty() {
            return Self::default();
        }

        durations.sort_unstable();
        let count = durations.len();
        let total: Duration = durations.iter().sum();

        Self {
            average: total / count as u32,
            p95: durations[(count * 95 / 100).min(count - 1)],
            max: durations[count - 1],
            count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UploadCounts {
    pub incremental: usize,
    pub incremental_bytes: u64,
    pub full: usize,
    pub full_bytes: u64,
}

impl UploadCounts {
    fn add(&mut self, upload: Upload) {
        match upload {
            Upload::None => {},
            Upload::Incremental { bytes } => {
                self.incremental += 1;
                self.incremental_bytes += bytes;
            },
            Upload::Full { bytes } => {
                self.full += 1;
                self.full_bytes += bytes;
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct FrameHistory {
    samples: VecDeque<FrameSample>,
    current: FrameSample,
    sprite_count: usize,
    views: Vec<ViewSprites>,
}

impl FrameHistory {
    pub fn begin_frame(&mut self, started: Instant) {
        self.current = FrameSample {
            started: Some(started),
            ..FrameSample::default()
        };
    }

    pub fn add(&mut self, span: EditorSpan, since: Instant) { self.current.editor[span as usize] += since.elapsed(); }

    pub fn measure<T>(&mut self, span: EditorSpan, f: impl FnOnce() -> T) -> T {
        let clock = Instant::now();
        let result = f();
        self.add(span, clock);

        result
    }

    pub fn record_renderer(&mut self, stats: &RenderStats) {
        for span in RenderSpan::ALL {
            self.current.renderer[span as usize] = stats.cpu(span);
        }

        self.current.gpu = stats
            .gpu
            .iter()
            .map(|pass| (Arc::clone(&pass.name), pass.duration))
            .collect();
        self.current.sprite_upload = stats.sprite_upload;
        self.current.lighting_upload = stats.lighting_upload;
        self.current.is_recorded = stats.is_recorded;
        self.sprite_count = stats.sprite_count;
        self.views.clone_from(&stats.views);
    }

    pub fn finish_frame(&mut self) {
        if self.samples.len() == HISTORY {
            self.samples.pop_front();
        }

        self.samples.push_back(std::mem::take(&mut self.current));
    }

    pub fn len(&self) -> usize { self.samples.len() }

    pub fn sprite_count(&self) -> usize { self.sprite_count }

    pub fn views(&self) -> &[ViewSprites] { &self.views }

    pub fn editor(&self, span: EditorSpan) -> Summary {
        Summary::of(self.samples.iter().map(|sample| sample.editor[span as usize]).collect())
    }

    pub fn renderer(&self, span: RenderSpan) -> Summary {
        Summary::of(
            self.samples
                .iter()
                .map(|sample| sample.renderer[span as usize])
                .collect(),
        )
    }

    /// Time from one frame's start to the next, so idle waits between frames count too.
    pub fn interval(&self) -> Summary {
        let starts = self
            .samples
            .iter()
            .filter_map(|sample| sample.started)
            .collect::<Vec<_>>();

        Summary::of(starts.windows(2).map(|pair| pair[1] - pair[0]).collect())
    }

    /// Passes in the order they first ran, each over the frames it ran in.
    pub fn gpu_passes(&self) -> Vec<(Arc<str>, Summary)> {
        let mut names: Vec<Arc<str>> = Vec::new();
        for (name, _) in self.samples.iter().flat_map(|sample| &sample.gpu) {
            if !names.contains(name) {
                names.push(Arc::clone(name));
            }
        }

        names
            .into_iter()
            .map(|name| {
                let durations = self
                    .samples
                    .iter()
                    .flat_map(|sample| &sample.gpu)
                    .filter(|(pass, _)| *pass == name)
                    .map(|(_, duration)| *duration)
                    .collect();

                (name, Summary::of(durations))
            })
            .collect()
    }

    pub fn gpu_total(&self) -> Summary {
        Summary::of(
            self.samples
                .iter()
                .filter(|sample| !sample.gpu.is_empty())
                .map(|sample| sample.gpu.iter().map(|(_, duration)| *duration).sum())
                .collect(),
        )
    }

    pub fn frame_millis(&self) -> Vec<f32> {
        self.samples
            .iter()
            .map(|sample| millis(sample.editor[EditorSpan::Frame as usize]))
            .collect()
    }

    pub fn gpu_millis(&self) -> Vec<f32> {
        self.samples
            .iter()
            .map(|sample| millis(sample.gpu.iter().map(|(_, duration)| *duration).sum()))
            .collect()
    }

    pub fn sprite_uploads(&self) -> UploadCounts {
        let mut counts = UploadCounts::default();
        self.samples.iter().for_each(|sample| counts.add(sample.sprite_upload));

        counts
    }

    pub fn lighting_uploads(&self) -> UploadCounts {
        let mut counts = UploadCounts::default();
        self.samples
            .iter()
            .for_each(|sample| counts.add(sample.lighting_upload));

        counts
    }

    pub fn records(&self) -> usize { self.samples.iter().filter(|sample| sample.is_recorded).count() }
}

pub fn millis(duration: Duration) -> f32 { duration.as_secs_f32() * 1000.0 }

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Summary;

    #[test]
    fn summaries_take_the_average_p95_and_max() {
        let summary = Summary::of((1..=100).rev().map(Duration::from_millis).collect());

        assert_eq!(summary.average, Duration::from_micros(50_500));
        assert_eq!(summary.p95, Duration::from_millis(96));
        assert_eq!(summary.max, Duration::from_millis(100));
        assert_eq!(summary.count, 100);
    }

    #[test]
    fn an_empty_summary_is_zero() {
        assert_eq!(Summary::of(Vec::new()), Summary::default());
    }
}
