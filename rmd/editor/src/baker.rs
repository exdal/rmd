use std::{
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
};

use editor::{
    Environment,
    bake::{Bake, BakeUpdate},
    document::DocumentId,
    progress::Progress,
};

use crate::loader::LoadView;

pub struct Request {
    pub document: DocumentId,
    pub path: String,
    pub environment: Arc<Environment>,
    pub job: Job,
}

pub enum Job {
    Build {
        atoms: Vec<vm::bake::Atom>,
        size: [i32; 3],
    },
    // grows a finished bake by the levels added since, without baking the rest again
    Extend {
        bake: Box<Bake>,
        level_count: u32,
        atoms: Vec<vm::bake::Atom>,
    },
}

pub enum Baked {
    Build(Option<Box<Bake>>),
    Extend(Box<Bake>, BakeUpdate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Build,
    Extend,
}

struct Active {
    document: DocumentId,
    path: String,
    kind: Kind,
    progress: Arc<Progress>,
    result: Receiver<Baked>,
}

#[derive(Default)]
pub struct Baker {
    active: Option<Active>,
    stale: Vec<DocumentId>,
}

impl Baker {
    pub fn is_busy(&self) -> bool { self.active.is_some() }

    pub fn baking(&self, document: DocumentId) -> bool {
        self.active.as_ref().is_some_and(|active| active.document == document)
    }

    pub fn extending(&self, document: DocumentId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.document == document && active.kind == Kind::Extend)
    }

    #[cfg(test)]
    pub fn invalidated(&self, document: DocumentId) -> bool { self.stale.contains(&document) }

    pub fn invalidate(&mut self, document: DocumentId) {
        if self.baking(document) && !self.stale.contains(&document) {
            self.stale.push(document);
        }
    }

    pub fn start(&mut self, request: Request) {
        let Request {
            document,
            path,
            environment,
            job,
        } = request;
        let progress = Arc::new(Progress::new());
        let worker = Arc::clone(&progress);
        let (sender, result) = channel();
        let kind = match job {
            Job::Build { .. } => Kind::Build,
            Job::Extend { .. } => Kind::Extend,
        };

        self.stale.retain(|stale| *stale != document);
        self.active = Some(Active {
            document,
            path,
            kind,
            progress,
            result,
        });

        thread::spawn(move || {
            let baked = match job {
                Job::Build { atoms, size } => {
                    Baked::Build(editor::bake::build_atoms(&environment, atoms, size, &worker).map(Box::new))
                },
                Job::Extend {
                    mut bake,
                    level_count,
                    atoms,
                } => {
                    let update = editor::bake::extend(&mut bake, &environment, level_count, atoms, &worker);
                    Baked::Extend(bake, update)
                },
            };

            let _ = sender.send(baked);
        });
    }

    pub fn poll(&mut self) -> Option<(DocumentId, Baked, bool)> {
        let (baked, is_lost) = match self.active.as_ref()?.result.try_recv() {
            Ok(baked) => (baked, false),
            Err(TryRecvError::Empty) => return None,
            // an extension took the document's bake with it
            Err(TryRecvError::Disconnected) => (Baked::Build(None), self.active.as_ref()?.kind == Kind::Extend),
        };

        let document = self.active.take()?.document;
        let outdated = is_lost || self.stale.contains(&document);
        self.stale.retain(|stale| *stale != document);

        Some((document, baked, outdated))
    }

    pub fn view(&self) -> Option<LoadView> {
        let active = self.active.as_ref()?;

        Some(LoadView {
            title: match active.kind {
                Kind::Build => "Baking appearances",
                Kind::Extend => "Baking new level",
            },
            path: active.path.clone(),
            snapshot: active.progress.snapshot(),
            cancelling: false,
            cancellable: false,
        })
    }
}
