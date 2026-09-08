//! Auxiliary test binary: attempt to open the SQLite storage in exclusive
//! mode from an independent OS process and report the outcome via exit code.
//!
//! This exists so `persistence_phase3` can validate the exclusive advisory
//! lock across *real* processes. Windows scopes `LockFileEx` locks to the
//! process rather than the handle, so a second `SqliteStorage::open()` from
//! the same process never contends there -- only a separate process does.
//!
//! Protocol:
//!   argv[1] = database path
//!   exit 0  = acquired the lock (after printing "LOCKED" and blocking on
//!             stdin until the parent closes it)
//!   exit 42 = StorageError::LockContention (someone else holds it)
//!   exit 1  = any other error (message on stderr)

use std::io::{Read, Write};

/// Does this `StorageError::Filesystem` message describe a contended file lock
/// rather than a real filesystem fault? `fs2::lock_contended_error()` is the
/// exact error a `try_lock_*` call returns on a contended file for this
/// platform; `SqliteStorage::open` formats it into the message with `{e}` and
/// `{e:?}` after the `flock failed:` prefix, so match on both its `Display`
/// and `Debug` renderings.
fn is_lock_contention_message(message: &str) -> bool {
    let contended = fs2::lock_contended_error();
    if !message.contains("flock failed") {
        return false;
    }
    message.contains(&contended.to_string()) || message.contains(&format!("{contended:?}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("db path argument is required");

    match amatl_core::SqliteStorage::open(
        std::path::Path::new(&path),
        amatl_core::config::SqliteLockingMode::Exclusive,
    )
    .await
    {
        Ok(_storage) => {
            // Signal the parent that the lock is held, then block until the
            // parent closes our stdin -- mirroring a live service that keeps
            // holding the lock for as long as it runs.
            println!("LOCKED");
            let _ = std::io::stdout().flush();
            let mut buf = [0u8; 1];
            let _ = std::io::stdin().read(&mut buf);
            std::process::exit(0);
        }
        Err(amatl_core::StorageError::LockContention) => std::process::exit(42),
        Err(amatl_core::StorageError::Filesystem { message })
            if is_lock_contention_message(&message) =>
        {
            // On Windows `try_lock_exclusive` reports a contended byte-range
            // lock as ERROR_LOCK_VIOLATION (os error 33), whose `io::ErrorKind`
            // is not `WouldBlock`, so `SqliteStorage::open` classifies it as a
            // generic `Filesystem` error rather than `LockContention`. It is
            // still contention -- recognise it here so the guarantee reads the
            // same on every platform.
            std::process::exit(42)
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "unexpected error: {e:?}");
            std::process::exit(1);
        }
    }
}
