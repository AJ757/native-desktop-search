//! Synchronisation and ingestion.
//!
//! One worker thread owns its own SQLite connection and is the only place
//! sync results are written. Crawls and file watchers only send it messages
//! over a channel. Each folder source gets:
//! - a recursive file watcher (debounced), started before the crawl so no
//!   change made during the crawl is lost;
//! - an initial crawl, which also sweeps away files deleted while the app
//!   was closed.
//!
//! Text extraction and indexing come in the Search stage; for now the worker
//! keeps the `documents` table in step with the disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify_debouncer_full::notify::event::ModifyKind;
use notify_debouncer_full::notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use rusqlite::Connection;

use crate::db::{self, DocumentUpsert};
use crate::error::{AppError, Result};
use crate::sources::{self, ExclusionFilter, LocalFolder, Source as _, SourceEvent};
use crate::types::{IndexStatus, Source, SourceStatus};

/// How long the watcher waits for a burst of events (an editor saving,
/// a folder being copied) to settle before reporting it.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// Work for the sync worker.
#[derive(Debug)]
pub enum SyncMsg {
    Events {
        source_id: i64,
        events: Vec<SourceEvent>,
    },
    Status {
        source_id: i64,
        status: SourceStatus,
    },
    /// A full crawl that began at `started_at` (Unix seconds) finished.
    CrawlDone { source_id: i64, started_at: i64 },
}

/// Handles shared by the service, crawl threads and watcher callbacks.
struct Shared {
    tx: Sender<SyncMsg>,
    /// File events sent to the worker but not yet written.
    queued: AtomicU32,
    /// Crawls currently running.
    active_crawls: AtomicU32,
}

impl Shared {
    fn send(&self, msg: SyncMsg) {
        let n = match &msg {
            SyncMsg::Events { events, .. } => u32::try_from(events.len()).unwrap_or(u32::MAX),
            _ => 0,
        };
        self.queued.fetch_add(n, Ordering::Relaxed);
        if self.tx.send(msg).is_err() {
            self.queued.fetch_sub(n, Ordering::Relaxed);
            tracing::error!("sync worker has stopped; change dropped");
        }
    }
}

/// A watched folder. Dropping it stops the watcher.
struct Running {
    _watcher: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
    cancel: Arc<AtomicBool>,
}

pub struct SyncService {
    shared: Arc<Shared>,
    running: Mutex<HashMap<i64, Running>>,
    /// Directories never indexed, such as the app's own data folder.
    excluded_roots: Vec<PathBuf>,
}

impl SyncService {
    /// Opens a second connection to `db_file` for the worker and starts it.
    /// The database runs in WAL mode, so this connection and the UI's one
    /// can be used at the same time.
    pub fn start(db_file: &Path, excluded_roots: Vec<PathBuf>) -> Result<Self> {
        let conn = db::open(db_file)?;
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            tx,
            queued: AtomicU32::new(0),
            active_crawls: AtomicU32::new(0),
        });
        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("sync-worker".into())
            .spawn(move || run_worker(conn, rx, worker_shared))?;
        Ok(Self {
            shared,
            running: Mutex::new(HashMap::new()),
            excluded_roots,
        })
    }

    /// Starts watching and crawling a source. Cloud sources are skipped
    /// until their connectors exist. Restarting a running source is allowed.
    pub fn start_source(&self, source: &Source, exclusion_patterns: &[String]) -> Result<()> {
        if !sources::is_folder_kind(source.kind) {
            tracing::debug!(
                id = source.id,
                kind = source.kind.as_str(),
                "no connector yet"
            );
            return Ok(());
        }
        self.stop_source(source.id);

        let root = match sources::folder_root_from_config(&source.config) {
            Ok(root) => root,
            Err(e) => {
                self.shared.send(SyncMsg::Status {
                    source_id: source.id,
                    status: SourceStatus::Error,
                });
                return Err(e);
            }
        };
        let filter = ExclusionFilter::new(exclusion_patterns, self.excluded_roots.clone());
        let folder = Arc::new(LocalFolder::new(source.kind, root, filter));
        let cancel = Arc::new(AtomicBool::new(false));

        // Watch first, then crawl, so changes made during the crawl are seen.
        let watcher = match watch_folder(source.id, &folder, &cancel, &self.shared) {
            Ok(w) => Some(w),
            Err(e) => {
                // The crawl still runs; only live updates are missing.
                // On Linux this is usually the inotify watch limit
                // (fs.inotify.max_user_watches).
                tracing::warn!(
                    id = source.id,
                    "live updates unavailable for this folder: {e}"
                );
                None
            }
        };
        spawn_crawl(source.id, &folder, &cancel, &self.shared)?;

        self.lock_running()?.insert(
            source.id,
            Running {
                _watcher: watcher,
                cancel,
            },
        );
        tracing::info!(id = source.id, root = %folder.root().display(), "watching folder");
        Ok(())
    }

    /// Stops the watcher and any crawl for a source. Does nothing if it is
    /// not running.
    pub fn stop_source(&self, source_id: i64) {
        let removed = match self.lock_running() {
            Ok(mut running) => running.remove(&source_id),
            Err(e) => {
                tracing::error!("{e}");
                None
            }
        };
        if let Some(r) = removed {
            r.cancel.store(true, Ordering::Relaxed);
            // Dropping `r` stops the watcher.
        }
    }

    pub fn status(&self, total_documents: u32) -> IndexStatus {
        let queued = self.shared.queued.load(Ordering::Relaxed);
        let crawling = self.shared.active_crawls.load(Ordering::Relaxed) > 0;
        IndexStatus {
            total_documents,
            queued,
            is_indexing: crawling || queued > 0,
        }
    }

    fn lock_running(&self) -> Result<std::sync::MutexGuard<'_, HashMap<i64, Running>>> {
        self.running
            .lock()
            .map_err(|_| AppError::Other("sync state lock poisoned".into()))
    }
}

fn watch_folder(
    source_id: i64,
    folder: &Arc<LocalFolder>,
    cancel: &Arc<AtomicBool>,
    shared: &Arc<Shared>,
) -> Result<Debouncer<RecommendedWatcher, RecommendedCache>> {
    let (folder_cb, cancel_cb, shared_cb) =
        (Arc::clone(folder), Arc::clone(cancel), Arc::clone(shared));
    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        if cancel_cb.load(Ordering::Relaxed) {
            return;
        }
        let events = match result {
            Ok(events) => events,
            Err(errors) => {
                for e in errors {
                    tracing::warn!(source_id, "file watcher error: {e}");
                }
                return;
            }
        };
        // The OS dropped events (queue overflow): only a full crawl is reliable.
        if events.iter().any(|e| e.need_rescan()) {
            tracing::info!(source_id, "watcher asked for a rescan");
            if let Err(e) = spawn_crawl(source_id, &folder_cb, &cancel_cb, &shared_cb) {
                tracing::error!(source_id, "could not start rescan: {e}");
            }
            return;
        }
        let mut changes = Vec::new();
        for event in &events {
            if matches!(event.kind, EventKind::Access(_)) {
                continue;
            }
            // Content or metadata edits never bring in a new directory, so
            // only creates, renames and unknown events walk directories.
            let walk_new_dirs = !matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
            );
            for path in &event.paths {
                changes.extend(folder_cb.changes_at(path, walk_new_dirs));
            }
        }
        if !changes.is_empty() {
            shared_cb.send(SyncMsg::Events {
                source_id,
                events: changes,
            });
        }
    })?;
    debouncer.watch(folder.root(), RecursiveMode::Recursive)?;
    Ok(debouncer)
}

/// Runs a full crawl of `folder` on its own thread.
fn spawn_crawl(
    source_id: i64,
    folder: &Arc<LocalFolder>,
    cancel: &Arc<AtomicBool>,
    shared: &Arc<Shared>,
) -> Result<()> {
    let (folder, cancel, shared) = (Arc::clone(folder), Arc::clone(cancel), Arc::clone(shared));
    shared.active_crawls.fetch_add(1, Ordering::Relaxed);
    shared.send(SyncMsg::Status {
        source_id,
        status: SourceStatus::Syncing,
    });
    let thread_shared = Arc::clone(&shared);
    let spawned = std::thread::Builder::new()
        .name(format!("crawl-{source_id}"))
        .spawn(move || {
            let shared = thread_shared;
            let started_at = now_secs();
            let result = folder.crawl(&cancel, &mut |events| {
                shared.send(SyncMsg::Events { source_id, events })
            });
            match result {
                Ok(true) => {
                    shared.send(SyncMsg::CrawlDone {
                        source_id,
                        started_at,
                    });
                    shared.send(SyncMsg::Status {
                        source_id,
                        status: SourceStatus::Idle,
                    });
                }
                Ok(false) => tracing::debug!(source_id, "crawl cancelled"),
                Err(e) => {
                    tracing::error!(source_id, "crawl failed: {e}");
                    shared.send(SyncMsg::Status {
                        source_id,
                        status: SourceStatus::Error,
                    });
                }
            }
            shared.active_crawls.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(e) = spawned {
        shared.active_crawls.fetch_sub(1, Ordering::Relaxed);
        return Err(e.into());
    }
    Ok(())
}

fn run_worker(mut conn: Connection, rx: Receiver<SyncMsg>, shared: Arc<Shared>) {
    for msg in rx {
        let n = match &msg {
            SyncMsg::Events { events, .. } => u32::try_from(events.len()).unwrap_or(u32::MAX),
            _ => 0,
        };
        if let Err(e) = apply(&mut conn, msg) {
            // Typically a source removed while its last events were queued.
            tracing::warn!("sync write failed: {e}");
        }
        shared.queued.fetch_sub(n, Ordering::Relaxed);
    }
}

/// Writes one message to the database. A batch of events is one transaction.
pub fn apply(conn: &mut Connection, msg: SyncMsg) -> Result<()> {
    match msg {
        SyncMsg::Events { source_id, events } => {
            let tx = conn.transaction()?;
            let now = now_secs();
            for event in &events {
                match event {
                    SourceEvent::Upsert {
                        external_id,
                        title,
                        mime,
                        modified_at,
                    } => db::upsert_document(
                        &tx,
                        source_id,
                        &DocumentUpsert {
                            external_id,
                            title,
                            mime: mime.as_deref(),
                            modified_at: *modified_at,
                        },
                        now,
                    )?,
                    SourceEvent::Delete { external_id } => {
                        db::mark_deleted_under(
                            &tx,
                            source_id,
                            external_id,
                            std::path::MAIN_SEPARATOR,
                        )?;
                    }
                }
            }
            tx.commit()?;
        }
        SyncMsg::Status { source_id, status } => {
            db::set_source_status(conn, source_id, status)?;
        }
        SyncMsg::CrawlDone {
            source_id,
            started_at,
        } => {
            let gone = db::sweep_unseen(conn, source_id, started_at)?;
            tracing::info!(source_id, removed = gone, "crawl finished");
        }
    }
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AddSourceRequest, SourceKind};
    use std::time::Instant;

    fn upsert(id: &str) -> SourceEvent {
        SourceEvent::Upsert {
            external_id: id.into(),
            title: id.into(),
            mime: None,
            modified_at: Some(1),
        }
    }

    fn conn_with_source() -> Connection {
        let conn = db::open_in_memory().unwrap();
        conn.execute("INSERT INTO sources (id, kind) VALUES (1, 'local')", [])
            .unwrap();
        conn
    }

    #[test]
    fn apply_writes_upserts_and_deletes() {
        let mut conn = conn_with_source();
        let sep = std::path::MAIN_SEPARATOR;
        let (a, b) = (format!("{sep}a"), format!("{sep}b"));
        apply(
            &mut conn,
            SyncMsg::Events {
                source_id: 1,
                events: vec![upsert(&a), upsert(&b)],
            },
        )
        .unwrap();
        apply(
            &mut conn,
            SyncMsg::Events {
                source_id: 1,
                events: vec![SourceEvent::Delete { external_id: a }],
            },
        )
        .unwrap();
        assert_eq!(db::count_documents(&conn).unwrap(), 1);
    }

    #[test]
    fn a_failed_batch_writes_nothing() {
        let mut conn = conn_with_source();
        // Source 2 does not exist, so the batch violates the foreign key.
        let err = apply(
            &mut conn,
            SyncMsg::Events {
                source_id: 2,
                events: vec![upsert("/a")],
            },
        );
        assert!(err.is_err());
        assert_eq!(db::count_documents(&conn).unwrap(), 0);
    }

    #[test]
    fn apply_sets_status() {
        let mut conn = conn_with_source();
        apply(
            &mut conn,
            SyncMsg::Status {
                source_id: 1,
                status: SourceStatus::Error,
            },
        )
        .unwrap();
        assert_eq!(
            db::get_source(&conn, 1).unwrap().status,
            SourceStatus::Error
        );
    }

    /// Polls `check` until it returns true or `timeout` passes.
    fn wait_until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if check() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// End to end with a real database file: add a folder, let the initial
    /// crawl run, and check that files deleted while "closed" are swept.
    /// The live watcher is not asserted on, because OS file events are too
    /// timing-dependent for CI.
    #[test]
    fn initial_crawl_fills_and_sweeps_the_database() {
        let data = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        std::fs::write(docs.path().join("a.txt"), "a").unwrap();
        std::fs::write(docs.path().join("b.md"), "b").unwrap();

        let db_file = data.path().join("app.db");
        let ui = db::open(&db_file).unwrap();
        let source = db::insert_source(
            &ui,
            &AddSourceRequest {
                kind: SourceKind::Local,
                config: serde_json::json!({ "path": docs.path() }),
            },
        )
        .unwrap();
        // A file from an earlier run that no longer exists on disk.
        db::upsert_document(
            &ui,
            source.id,
            &DocumentUpsert {
                external_id: &sources::path_key(&docs.path().join("gone.txt")),
                title: "gone.txt",
                mime: None,
                modified_at: Some(1),
            },
            0,
        )
        .unwrap();

        let sync = SyncService::start(&db_file, vec![data.path().to_path_buf()]).unwrap();
        sync.start_source(&source, &[]).unwrap();

        let settled = wait_until(Duration::from_secs(10), || {
            !sync.status(0).is_indexing
                && db::get_source(&ui, source.id).unwrap().status == SourceStatus::Idle
                && db::count_documents(&ui).unwrap() == 2
        });
        assert!(settled, "crawl did not finish: {:?}", sync.status(0));
        sync.stop_source(source.id);
    }

    #[test]
    fn a_source_with_a_missing_folder_is_marked_error() {
        let data = tempfile::tempdir().unwrap();
        let db_file = data.path().join("app.db");
        let ui = db::open(&db_file).unwrap();
        let source = db::insert_source(
            &ui,
            &AddSourceRequest {
                kind: SourceKind::Local,
                config: serde_json::json!({ "path": data.path().join("missing") }),
            },
        )
        .unwrap();

        let sync = SyncService::start(&db_file, Vec::new()).unwrap();
        assert!(sync.start_source(&source, &[]).is_err());
        assert!(wait_until(Duration::from_secs(5), || {
            db::get_source(&ui, source.id).unwrap().status == SourceStatus::Error
        }));
    }
}
