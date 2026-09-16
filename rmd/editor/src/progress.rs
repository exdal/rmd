use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Preprocess,
    Parse,
    Analyze,
    Icons,
    Textures,
    Thumbnails,
    FindMaps,
    ReadMap,
    ParseMap,
    Instantiate,
    Initialize,
    Prepare,
    Light,
    Smooth,
}

impl Stage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preprocess => "Preprocessing",
            Self::Parse => "Parsing",
            Self::Analyze => "Analyzing",
            Self::Icons => "Reading icons",
            Self::Textures => "Building textures",
            Self::Thumbnails => "Building thumbnails",
            Self::FindMaps => "Finding maps",
            Self::ReadMap => "Reading map",
            Self::ParseMap => "Parsing map",
            Self::Instantiate => "Instantiating atoms",
            Self::Initialize => "Running demir_initialize",
            Self::Prepare => "Preparing atoms",
            Self::Light => "Collecting lights",
            Self::Smooth => "Baking appearances",
        }
    }

    pub const fn from_bake(stage: vm::bake::Stage) -> Self {
        match stage {
            vm::bake::Stage::Instantiate => Self::Instantiate,
            vm::bake::Stage::Initialize => Self::Initialize,
            vm::bake::Stage::Prepare => Self::Prepare,
            vm::bake::Stage::Light => Self::Light,
            vm::bake::Stage::Smooth => Self::Smooth,
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Preprocess => 0,
            Self::Parse => 1,
            Self::Analyze => 2,
            Self::Icons => 3,
            Self::Textures => 4,
            Self::Thumbnails => 5,
            Self::FindMaps => 6,
            Self::ReadMap => 7,
            Self::ParseMap => 8,
            Self::Instantiate => 9,
            Self::Initialize => 10,
            Self::Prepare => 11,
            Self::Light => 12,
            Self::Smooth => 13,
        }
    }

    const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Parse,
            2 => Self::Analyze,
            3 => Self::Icons,
            4 => Self::Textures,
            5 => Self::Thumbnails,
            6 => Self::FindMaps,
            7 => Self::ReadMap,
            8 => Self::ParseMap,
            9 => Self::Instantiate,
            10 => Self::Initialize,
            11 => Self::Prepare,
            12 => Self::Light,
            13 => Self::Smooth,
            _ => Self::Preprocess,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub stage: Stage,
    pub done: usize,
    pub total: usize,
    pub detail: String,
}

impl Snapshot {
    pub fn fraction(&self) -> Option<f32> {
        (self.total > 0).then(|| (self.done.min(self.total) as f32) / (self.total as f32))
    }
}

#[derive(Debug, Default)]
pub struct Progress {
    stage: AtomicU8,
    done: AtomicUsize,
    total: AtomicUsize,
    detail: Mutex<String>,
    cancelled: AtomicBool,
}

impl Progress {
    pub fn new() -> Self { Self::default() }

    pub fn enter(&self, stage: Stage, total: usize) {
        self.stage.store(stage.code(), Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.set_detail("");
    }

    pub fn advance(&self, detail: &str) {
        self.done.fetch_add(1, Ordering::Relaxed);
        self.set_detail(detail);
    }

    pub fn set_done(&self, done: usize) { self.done.store(done, Ordering::Relaxed); }

    pub fn set_detail(&self, detail: &str) {
        if let Ok(mut current) = self.detail.lock() {
            current.clear();
            current.push_str(detail);
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            stage: Stage::from_code(self.stage.load(Ordering::Relaxed)),
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            detail: self.detail.lock().map(|detail| detail.clone()).unwrap_or_default(),
        }
    }

    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Relaxed); }

    pub fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::Relaxed) }
}

#[cfg(test)]
mod tests {
    use super::{Progress, Stage};

    #[test]
    fn stage_codes_round_trip() {
        for stage in [
            Stage::Preprocess,
            Stage::Parse,
            Stage::Analyze,
            Stage::Icons,
            Stage::Textures,
            Stage::Thumbnails,
            Stage::FindMaps,
            Stage::ReadMap,
            Stage::ParseMap,
            Stage::Instantiate,
            Stage::Initialize,
            Stage::Prepare,
            Stage::Light,
            Stage::Smooth,
        ] {
            let progress = Progress::new();
            progress.enter(stage, 0);

            assert_eq!(progress.snapshot().stage, stage);
        }
    }

    #[test]
    fn entering_a_stage_resets_the_counter() {
        let progress = Progress::new();
        progress.enter(Stage::Icons, 4);
        progress.advance("icons/one.dmi");
        progress.advance("icons/two.dmi");

        let snapshot = progress.snapshot();
        assert_eq!((snapshot.done, snapshot.total), (2, 4));
        assert_eq!(snapshot.detail, "icons/two.dmi");
        assert_eq!(snapshot.fraction(), Some(0.5));

        progress.enter(Stage::Parse, 0);
        let snapshot = progress.snapshot();
        assert_eq!((snapshot.done, snapshot.total), (0, 0));
        assert!(snapshot.detail.is_empty());
        assert_eq!(snapshot.fraction(), None);
    }

    #[test]
    fn cancellation_is_visible_to_the_worker() {
        let progress = Progress::new();
        assert!(!progress.is_cancelled());

        progress.cancel();
        assert!(progress.is_cancelled());
    }
}
