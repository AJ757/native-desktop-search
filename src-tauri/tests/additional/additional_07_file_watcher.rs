use crate::support;
use native_desktop_search_lib::{db, sync::SyncService};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
use support::*;

#[test]
fn filesystem_create_event_is_written_to_sqlite() {
    let tmp = temp();
    let root = tmp.path().join("watched");
    fs::create_dir(&root).unwrap();
    let db_path = tmp.path().join("watcher.sqlite");
    let conn = db::open(&db_path).unwrap();
    let src = source(&conn, &root);
    let service = SyncService::start(&db_path, Vec::new()).unwrap();
    service.start_source(&src, &[]).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while service.status(0).is_indexing && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    let added = root.join("created-after-watch.txt");
    fs::write(&added, "watcher event").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut indexed = false;
    while Instant::now() < deadline {
        indexed = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM documents WHERE external_id=?1 AND deleted=0)",
                [added.to_string_lossy().as_ref()],
                |row| row.get::<_, bool>(0),
            )
            .unwrap();
        if indexed {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    service.stop_source(src.id);
    assert!(indexed, "watcher did not persist the newly created file");
}
