//! `RusqliteStorage` must pass the same `Storage` conformance suite
//! the fake passes.

use panops_core::conformance::storage::run_suite;
use panops_portable::rusqlite_storage::RusqliteStorage;

#[test]
fn rusqlite_storage_passes_conformance() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("panops.db");
    let storage = RusqliteStorage::new(&db).expect("open fresh DB");
    run_suite(&storage);
}

#[test]
fn rusqlite_storage_rejects_unknown_schema_version() {
    use rusqlite::Connection;
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("panops.db");
    {
        let conn = Connection::open(&db).unwrap();
        // Pretend a future version wrote this DB.
        conn.execute_batch("PRAGMA user_version = 999;").unwrap();
    }
    let result = RusqliteStorage::new(&db);
    match result {
        Ok(_) => panic!("future version should be rejected"),
        Err(e) => assert!(format!("{e}").contains("schema mismatch"), "got: {e}"),
    }
}
