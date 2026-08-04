//! The listener. Everything it decides lives in the library beside it.
//!
//! Configuration is environment variables, because that is what a container
//! manager hands a process:
//!
//! - `BRAIN_VECTORS_TOKEN` — the shared secret. **Required**: a service with
//!   no token is one anything on the network can fill with plausible nonsense
//!   or read a vault out of, and refusing to start is the only way that
//!   failure is loud.
//! - `BRAIN_VECTORS_ADDR` — where to listen, default `0.0.0.0:8082`.
//! - `BRAIN_VECTORS_DATA` — the data directory, default `/var/lib/brain-vectors`.
//!   The vectors are a file in it and the vault is the `vault/` folder beside
//!   them — real Markdown, so whatever backs up that directory backs up the
//!   notes, and `git init` in it gives history for free.

use std::io::{BufReader, BufWriter};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brain_server::{default_store_path, http, notes, route, Store};

/// A client that opens a connection and then says nothing must not hold a
/// thread for ever.
const TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    // The image carries no curl and no wget, so the health check is the binary
    // asking itself. `/health` needs no token, which is what makes this work
    // without handing the secret to the container manager as well.
    if std::env::args().any(|argument| argument == "--health") {
        std::process::exit(if healthy() { 0 } else { 1 });
    }

    let Ok(token) = std::env::var("BRAIN_VECTORS_TOKEN") else {
        eprintln!("BRAIN_VECTORS_TOKEN is not set — refusing to start without one");
        std::process::exit(2);
    };
    if token.len() < 32 {
        eprintln!("BRAIN_VECTORS_TOKEN is shorter than 32 characters — refusing to start");
        std::process::exit(2);
    }
    let address =
        std::env::var("BRAIN_VECTORS_ADDR").unwrap_or_else(|_| "0.0.0.0:8082".to_string());
    let path = default_store_path();

    let store = Store::load(&path);
    let vault_root = path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("vault");
    if let Err(error) = std::fs::create_dir_all(&vault_root) {
        eprintln!("could not make {}: {error}", vault_root.display());
        std::process::exit(1);
    }
    let vault = Arc::new(notes::Vault::new(PathBuf::from(&vault_root)));
    println!(
        "brain-server: {} vectors from {}, {} notes in {}",
        store.len(),
        path.display(),
        vault.list().len(),
        vault_root.display()
    );
    let store = Arc::new(Mutex::new(store));

    let listener = match TcpListener::bind(&address) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("could not listen on {address}: {error}");
            std::process::exit(1);
        }
    };
    println!("brain-vectors: listening on {address}");

    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            continue;
        };
        // A thread per connection. The client count is the number of machines
        // one person owns, and a pool would be more machinery than the problem.
        let store = Arc::clone(&store);
        let vault = Arc::clone(&vault);
        let token = token.clone();
        let path = path.clone();
        std::thread::spawn(move || serve(stream, &token, &store, &vault, &path));
    }
}

/// Ask `/health` over the loopback and read the status line.
fn healthy() -> bool {
    use std::io::{BufRead, BufReader, Write};

    let address =
        std::env::var("BRAIN_VECTORS_ADDR").unwrap_or_else(|_| "0.0.0.0:8082".to_string());
    // Whatever it is bound to, this end talks to it over the loopback: the
    // check is asking whether *this* container is serving, not whether the
    // network can reach it.
    let port = address.rsplit(':').next().unwrap_or("8082");
    let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{port}")) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut status = String::new();
    BufReader::new(stream).read_line(&mut status).is_ok() && status.starts_with("HTTP/1.1 200")
}

fn serve(
    stream: TcpStream,
    token: &str,
    store: &Mutex<Store>,
    vault: &notes::Vault,
    path: &std::path::Path,
) {
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut input = BufReader::new(stream);
    let mut output = BufWriter::new(write_half);

    let (status, body) = match http::read_request(&mut input) {
        Ok(request) => {
            let writing = request.path == "/publish";
            // The lock is held across the whole route and the save, which
            // serialises the note writes as well as the vector ones. That is
            // deliberate rather than incidental: two clients writing the same
            // note at once would otherwise both read the same current hash,
            // both find it matches their base, and both write — which is the
            // one way a stale-write check can be got round. One person's
            // machines are not a throughput problem.
            let mut store = match store.lock() {
                Ok(store) => store,
                // A panicked thread poisoned it. The store is a cache of
                // append-only entries, so carrying on with it is safer than
                // taking the service down.
                Err(poisoned) => poisoned.into_inner(),
            };
            let answer = route(&request, token, &mut store, vault);
            if writing && answer.0 == 200 {
                if let Err(error) = store.save(path) {
                    // In memory and serving; the next publish writes again.
                    eprintln!("brain-server: could not save the store: {error}");
                }
            }
            answer
        }
        // Nothing was asked, so there is nothing to answer.
        Err(http::Bad::Closed) => return,
        Err(http::Bad::TooLarge) => (413, b"{\"error\":\"too large\"}".to_vec()),
        Err(http::Bad::Malformed) => (400, b"{\"error\":\"bad request\"}".to_vec()),
    };
    let _ = http::respond(&mut output, status, &body);
}
