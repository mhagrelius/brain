//! Drive a real `brain-server` with the real client, over a real socket.
//!
//! The two wire formats are defined in crates that never see each other's
//! types. Each has a test pinning the exact bytes, which catches drift — but
//! only this catches the case where both are self-consistent and disagree.
//!
//!     BRAIN_VECTORS_TOKEN=… BRAIN_VECTORS_DATA=/tmp/x \
//!       BRAIN_VECTORS_ADDR=127.0.0.1:18090 cargo run --release -p brain-server &
//!     cargo run --example sync_check -- http://127.0.0.1:18090 $TOKEN
//!
//! It writes only into temporary directories of its own and into the vault the
//! server was pointed at, so give it a server started for the purpose rather
//! than the one holding your notes.

use brain::model::note::NoteId;
use brain::model::sync::{self, Hash, Remote, Snapshot};
use brain::model::vault::Vault;
use brain::ui::VaultServer;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let (Some(url), Some(token)) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: sync_check <url> <token>");
        std::process::exit(2);
    };
    let server = VaultServer::new(&url, &token);

    match server.list() {
        Ok(notes) => println!("connected: {} notes on the server\n", notes.len()),
        Err(error) => {
            eprintln!("could not reach {url}: {error}");
            std::process::exit(1);
        }
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let name = format!("sync-check-{stamp}.md");

    // Two vaults standing in for two machines.
    let (one_dir, one) = vault();
    let (two_dir, two) = vault();
    std::fs::write(one_dir.path().join(&name), "written on machine one").expect("seed");

    let (one_base, report) = pass(&one, &Snapshot::new(), &server);
    check("machine one pushed its note", report.pushed == 1);

    let (two_base, report) = pass(&two, &Snapshot::new(), &server);
    // At least one, not exactly one: a server that already holds notes is the
    // normal case for a real deployment, and the first pass pulls all of them.
    // Asserting an exact count here made this fail against a working server,
    // which is the wrong way round for a check.
    check("machine two pulled it", report.pulled >= 1);
    check(
        "and it landed as a real file with the right text",
        text_at(two_dir.path().join(&name)).as_deref() == Some("written on machine one"),
    );

    // Both edit it, neither having seen the other.
    std::fs::write(one_dir.path().join(&name), "one's second thoughts").expect("edit");
    std::fs::write(two_dir.path().join(&name), "two's second thoughts").expect("edit");

    let (_, report) = pass(&one, &one_base, &server);
    check("machine one's edit went up", report.pushed == 1);

    let (_, report) = pass(&two, &two_base, &server);
    check("machine two found a conflict", report.conflicted == 1);
    check(
        "its own version was left exactly as it was",
        text_at(two_dir.path().join(&name)).as_deref() == Some("two's second thoughts"),
    );
    let copy = sync::conflict_id(&NoteId::from_relative(name.clone()), "server", "check");
    check(
        "and the other version is a note beside it",
        two_dir.path().join(copy.as_str()).exists(),
    );

    // Tidy up after ourselves on the server.
    let hash = Hash::of("one's second thoughts");
    let id = NoteId::from_relative(name);
    check(
        "the note can be deleted again",
        matches!(server.delete(&id, Some(hash)), sync::Put::Done(_)),
    );

    println!("\nthe client and the server agree.");
}

fn vault() -> (tempfile::TempDir, Vault) {
    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Vault::new(directory.path().to_path_buf());
    (directory, vault)
}

fn pass(vault: &Vault, base: &Snapshot, server: &VaultServer) -> (Snapshot, sync::Report) {
    sync::run(vault, base, server, "server", "check").expect("a pass")
}

fn text_at(path: std::path::PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn check(what: &str, held: bool) {
    println!("{} {what}", if held { "ok  " } else { "FAIL" });
    if !held {
        std::process::exit(1);
    }
}
