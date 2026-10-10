use std::{
    collections::{BTreeMap, VecDeque},
    mem,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
    sync::mpsc,
    thread,
    time::Instant,
};

use editor::{Environment, tool::Tool};
use net::{Client, CodebaseId, Event, PasswordHash, Server, ServerConfig};

use super::{Coop, CoopConnection, CoopStatus, Prepared, SharedState};
use crate::session::Session;

impl Session {
    fn loaded_codebase_id(&self) -> Result<CodebaseId, String> {
        let environment = self.state.environment.as_ref().ok_or("open a codebase first")?;
        Ok(CodebaseId {
            hash: environment.fingerprint().map_err(|error| error.to_string())?,
            git_hint: environment.git_hint(),
        })
    }

    pub fn host_coop(&mut self, port: u16, password: PasswordHash, nick: String) -> Result<(), String> {
        let codebase = self.loaded_codebase_id()?;
        let server = Server::spawn(ServerConfig {
            bind: SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
            password,
            codebase: Some(codebase.clone()),
        })
        .map_err(|e| format!("could not host on port {port}: {e}"))?;

        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, server.local_addr().port())).to_string();
        let connection = CoopConnection::Host {
            port: server.local_addr().port(),
            password,
        };
        self.start_coop(addr, password, nick, codebase, connection, Some(server))
    }

    pub fn join_coop(&mut self, addr: String, password: PasswordHash, nick: String) -> Result<(), String> {
        let codebase = self.loaded_codebase_id()?;
        let connection = CoopConnection::Join {
            addr: addr.clone(),
            password,
        };
        self.start_coop(addr, password, nick, codebase, connection, None)
    }

    fn start_coop(
        &mut self, addr: String, password: PasswordHash, nick: String, codebase: CodebaseId,
        connection: CoopConnection, server: Option<Server>,
    ) -> Result<(), String> {
        let environment_root = self.environment_path().ok_or("open a codebase first")?.to_path_buf();
        self.leave_coop();
        let client = Client::connect(addr, password, nick.clone(), codebase.clone());
        self.coop = Some(Coop {
            status: CoopStatus::Connecting,
            nick,
            you: None,
            peers: BTreeMap::new(),
            server,
            codebase,
            connection,
            environment_root,
            paused: false,
            needs_cleanup: false,
            stopping_server: None,
            comments: BTreeMap::new(),
            shared_maps: BTreeMap::new(),
            activity: VecDeque::new(),
            following: None,
            prepared: mpsc::channel(),
            client: Some(client),
            last_poll: Instant::now(),
        });

        Ok(())
    }

    pub fn retry_coop(&mut self) -> Result<(), String> {
        let coop = self.coop.as_ref().ok_or("no co-op attempt to retry")?;
        if !coop.can_retry() {
            return Err(String::from("wait for the previous co-op attempt to finish"));
        }

        let nick = coop.nick.clone();
        match coop.connection.clone() {
            CoopConnection::Host { port, password } => self.host_coop(port, password, nick),
            CoopConnection::Join { addr, password } => self.join_coop(addr, password, nick),
        }
    }

    pub fn begin_coop_reload(&mut self) {
        if let Some(coop) = self.coop.as_mut() {
            coop.paused = true;
        }

        self.lock_coop_documents();
    }

    pub fn finish_coop_reload(&mut self) {
        if let Some(coop) = self.coop.as_mut() {
            coop.paused = false;
        }

        self.lock_coop_documents();
    }

    pub(crate) fn check_coop_codebase(&mut self, environment: &Environment) {
        if let Some(coop) = self.coop.as_mut() {
            let local = environment.fingerprint().map(|hash| CodebaseId {
                hash,
                git_hint: environment.git_hint(),
            });
            if matches!(coop.status, CoopStatus::Connecting | CoopStatus::Connected) {
                match &local {
                    Ok(local) if local.hash != coop.codebase.hash => coop.mismatch(coop.codebase.clone()),
                    Ok(_) if environment.root != coop.environment_root => {
                        coop.end(String::from("codebase switched; co-op disconnected"));
                    },
                    Err(error) => coop.end(format!("cannot verify the loaded codebase: {error}")),
                    _ => {},
                }
            }

            if let Ok(local) = local {
                coop.codebase = local;
            }
        }

        self.finish_coop_reload();
        self.clean_ended_coop();
    }

    fn clean_ended_coop(&mut self) {
        let Some(coop) = self.coop.as_mut().filter(|coop| coop.needs_cleanup) else {
            return;
        };

        coop.needs_cleanup = false;
        let pending = mem::take(&mut coop.shared_maps)
            .into_values()
            .filter_map(|map| map.pending_document)
            .collect::<Vec<_>>();
        coop.prepared = mpsc::channel();

        for id in pending {
            self.close_map(id);
        }

        self.lock_coop_documents();
    }

    pub fn leave_coop(&mut self) {
        let Some(mut coop) = self.coop.take() else {
            return;
        };

        // a server waits for its peers to hear the close, keep that off the frame
        if let Some(server) = coop.server.take() {
            thread::spawn(move || drop(server));
        }

        for id in coop
            .shared_maps
            .into_values()
            .filter_map(|shared_map| shared_map.pending_document)
        {
            self.close_map(id);
        }

        self.lock_coop_documents();
    }

    pub fn poll_coop(&mut self) {
        if self.state.tool == Tool::Comment && !self.comment_tool_available() {
            self.state.tool = Tool::Select;
        }

        let codebase = self.codebase_dir().map(Path::to_path_buf);
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        if coop
            .stopping_server
            .as_ref()
            .is_some_and(|stopping| !matches!(stopping.try_recv(), Err(mpsc::TryRecvError::Empty)))
        {
            coop.stopping_server = None;
        }

        if coop.paused {
            return;
        }

        let events = coop
            .client
            .as_ref()
            .map(|client| client.poll().collect::<Vec<_>>())
            .unwrap_or_default();

        // in order, so a share arriving after a stop in the same batch still applies
        for event in events {
            match event {
                Event::MapUnshared { path, by } => self.forget_coop_map(&path, by),
                event => {
                    if let Some(coop) = self.coop.as_mut() {
                        coop.apply(event, codebase.as_deref());
                    }
                },
            }
        }

        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        if !coop.is_connected() {
            self.clean_ended_coop();
            return;
        }

        let prepared = coop.prepared.1.try_iter().collect::<Vec<_>>();

        let now = Instant::now();
        let elapsed = now.duration_since(coop.last_poll).as_secs_f32();
        coop.last_poll = now;
        for peer in coop.peers.values_mut() {
            peer.follow(elapsed);
            peer.expire(now);
        }

        let mut waiting = Vec::new();
        for shared_map in coop.shared_maps.values_mut() {
            let SharedState::Waiting(received) = &shared_map.state else {
                continue;
            };

            let loading = SharedState::Loading(received.generation);
            if let SharedState::Waiting(received) = mem::replace(&mut shared_map.state, loading) {
                waiting.push(*received);
            }
        }

        for received in waiting {
            self.open_shared_map(received);
        }

        for prepared in prepared {
            match prepared {
                Prepared::Upload {
                    path,
                    base,
                    levels,
                    bytes,
                } => {
                    if let Some(client) = self.collaborating_client() {
                        client.share_map(path, base, levels, bytes);
                    }
                },
                Prepared::Snapshot(received) => self.open_shared_map(received),
                Prepared::Unreadable { path, generation } => {
                    if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path))
                        && matches!(shared_map.state, SharedState::Loading(loading) if loading == generation)
                    {
                        shared_map.restart(SharedState::Closed);
                    }
                },
            }
        }

        self.follow_shared_documents();
        self.apply_coop_edits();
        self.send_coop_edits();
    }
}
