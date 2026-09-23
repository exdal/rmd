use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender, channel},
    thread,
};

use editor::{
    document::DocumentId,
    git::{CommitRef, GitError, Operation, RepoPath},
};

pub struct Status {
    pub branch: String,
    pub head: Option<CommitRef>,
    pub operation: Option<Operation>,
    pub unmerged: bool,
}

pub enum Outcome {
    Status(Result<Box<Status>, GitError>),
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
}

impl Default for GitWorker {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            generations: HashMap::new(),
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
                Ok(Box::new(Status {
                    branch,
                    head,
                    operation,
                    unmerged,
                }))
            })();
            let _ = sender.send(Finished {
                document,
                kind: 0,
                generation,
                outcome: Outcome::Status(result),
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

    pub fn poll(&mut self) -> Vec<Finished> {
        let mut finished = Vec::new();
        while let Ok(result) = self.receiver.try_recv() {
            if self.generations.get(&(result.document, result.kind)) == Some(&result.generation) {
                finished.push(result);
            }
        }
        finished
    }

    pub fn close(&mut self, document: DocumentId) {
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
