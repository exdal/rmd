use std::{
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
};

use editor::{Environment, bake::Bake, document::DocumentId, progress::Progress};

use crate::loader::LoadView;

pub struct Request {
    pub document: DocumentId,
    pub path: String,
    pub environment: Arc<Environment>,
    pub atoms: Vec<vm::bake::Atom>,
    pub size: [i32; 3],
}

struct Active {
    document: DocumentId,
    path: String,
    progress: Arc<Progress>,
    result: Receiver<Option<Bake>>,
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
            atoms,
            size,
        } = request;
        let progress = Arc::new(Progress::new());
        let worker = Arc::clone(&progress);
        let (sender, result) = channel();

        self.stale.retain(|stale| *stale != document);
        self.active = Some(Active {
            document,
            path,
            progress,
            result,
        });

        thread::spawn(move || {
            let _ = sender.send(editor::bake::build_atoms(&environment, atoms, size, &worker));
        });
    }

    pub fn poll(&mut self) -> Option<(DocumentId, Option<Bake>, bool)> {
        let bake = match self.active.as_ref()?.result.try_recv() {
            Ok(bake) => bake,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };

        let document = self.active.take()?.document;
        let outdated = self.stale.contains(&document);
        self.stale.retain(|stale| *stale != document);

        Some((document, bake, outdated))
    }

    pub fn view(&self) -> Option<LoadView> {
        let active = self.active.as_ref()?;

        Some(LoadView {
            title: "Baking appearances",
            path: active.path.clone(),
            snapshot: active.progress.snapshot(),
            cancelling: false,
            cancellable: false,
        })
    }
}
