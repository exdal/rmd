use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
};

use dmm::Map;
use editor::{
    blame::{self, BlameResult, GitVersions},
    document::DocumentId,
    git::{CommitRef, GitError, Operation, RepoPath},
};

pub struct Status {
    pub branch: String,
    pub head: Option<CommitRef>,
    pub operation: Option<Operation>,
    pub unmerged: bool,
    pub web_commit_base: Option<String>,
}

pub enum Outcome {
    Status(Result<Status, GitError>),
    Blame {
        revision: u64,
        snapshot: Map,
        result: Result<BlameResult, GitError>,
    },
    MarkResolved(Result<(), GitError>),
}

pub struct Finished {
    pub document: DocumentId,
    kind: u8,
    generation: u64,
    pub outcome: Outcome,
}

pub struct GitWorker {
    sender: Sender<Finished>,
    receiver: Receiver<Finished>,
    generations: HashMap<(DocumentId, u8), u64>,
    cancels: HashMap<DocumentId, Arc<AtomicBool>>,
    progress: HashMap<DocumentId, Arc<AtomicUsize>>,
}

impl Default for GitWorker {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            generations: HashMap::new(),
            cancels: HashMap::new(),
            progress: HashMap::new(),
        }
    }
}

impl GitWorker {
    fn next(&mut self, document: DocumentId, kind: u8) -> (u64, Sender<Finished>) {
        let generation = self.generations.entry((document, kind)).or_default();
        *generation = generation.wrapping_add(1);
        (*generation, self.sender.clone())
    }

    pub fn status(&mut self, document: DocumentId, path: RepoPath) {
        let (generation, sender) = self.next(document, 0);
        thread::spawn(move || {
            let result = (|| {
                let repo = path.open()?;
                let branch = repo.branch()?;
                let head = repo.head_ref();
                let operation = repo.operation();
                let unmerged = repo.unmerged()?.is_some();
                let web_commit_base = repo.web_commit_base();
                Ok(Status {
                    branch,
                    head,
                    operation,
                    unmerged,
                    web_commit_base,
                })
            })();
            let _ = sender.send(Finished {
                document,
                kind: 0,
                generation,
                outcome: Outcome::Status(result),
            });
        });
    }

    pub fn blame(&mut self, document: DocumentId, path: RepoPath, snapshot: Map, revision: u64, depth: usize) {
        if let Some(previous) = self.cancels.insert(document, Arc::new(AtomicBool::new(false))) {
            previous.store(true, Ordering::Relaxed);
        }

        let cancelled = Arc::clone(&self.cancels[&document]);
        let progress = Arc::new(AtomicUsize::new(0));
        self.progress.insert(document, Arc::clone(&progress));
        let (generation, sender) = self.next(document, 1);

        thread::spawn(move || {
            let result = GitVersions::open_with_cancel(path, depth, &|| cancelled.load(Ordering::Relaxed)).and_then(
                |mut versions| {
                    blame::blame(
                        &snapshot,
                        &mut versions,
                        &|| cancelled.load(Ordering::Relaxed),
                        &mut |done, _| progress.store(done, Ordering::Relaxed),
                    )
                },
            );
            let _ = sender.send(Finished {
                document,
                kind: 1,
                generation,
                outcome: Outcome::Blame {
                    revision,
                    snapshot,
                    result,
                },
            });
        });
    }

    pub fn mark_resolved(&mut self, document: DocumentId, path: RepoPath) {
        let (generation, sender) = self.next(document, 2);
        thread::spawn(move || {
            let result = path.open().and_then(|repo| repo.mark_resolved());
            let _ = sender.send(Finished {
                document,
                kind: 2,
                generation,
                outcome: Outcome::MarkResolved(result),
            });
        });
    }

    pub fn blame_progress(&self, document: DocumentId) -> Option<usize> {
        self.progress.get(&document).map(|value| value.load(Ordering::Relaxed))
    }

    pub fn poll(&mut self) -> Vec<Finished> {
        let mut finished = Vec::new();
        while let Ok(result) = self.receiver.try_recv() {
            if self.generations.get(&(result.document, result.kind)) == Some(&result.generation) {
                if result.kind == 1 {
                    self.progress.remove(&result.document);
                    self.cancels.remove(&result.document);
                }
                finished.push(result);
            }
        }
        finished
    }

    pub fn close(&mut self, document: DocumentId) {
        if let Some(cancelled) = self.cancels.remove(&document) {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.progress.remove(&document);
        // Keep the counters so an old result cannot match a new job if this document is reopened.
        for ((id, _), generation) in &mut self.generations {
            if *id == document {
                *generation = generation.wrapping_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use editor::document::DocumentId;

    use super::{Finished, GitWorker, Outcome};

    #[test]
    fn closing_a_document_rejects_results_from_its_previous_jobs() {
        let mut worker = GitWorker::default();
        let document = DocumentId::new();
        let (old_generation, old_sender) = worker.next(document, 2);
        worker.close(document);
        let (new_generation, new_sender) = worker.next(document, 2);
        assert_ne!(old_generation, new_generation);

        for (generation, sender) in [(old_generation, old_sender), (new_generation, new_sender)] {
            sender
                .send(Finished {
                    document,
                    kind: 2,
                    generation,
                    outcome: Outcome::MarkResolved(Ok(())),
                })
                .unwrap();
        }

        assert_eq!(worker.poll().len(), 1);
    }
}
