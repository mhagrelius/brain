//! A shared vector store: the wire format, the store, and the routing.
//!
//! Brain embeds a vault on whichever machine opened it first. Without this,
//! every other machine pays the same minutes again for text that hashed
//! identically. With it, the second machine fetches and the model is never
//! called.
//!
//! # Why this is the first thing on a network
//!
//! Vectors are the one derived thing Brain keeps that it cannot rebuild
//! cheaply — and they are still *derived*. A store that is unreachable, wrong,
//! or wiped costs one pass of embedding and nothing else; search carries on
//! lexically in the meantime. Notes do not have that property, which is why
//! they wait until this has proved the container, the network path and the
//! client's tolerance of a server that is not there.
//!
//! # The model is part of the key
//!
//! Vectors from two models rank plausibly and wrongly, which is the worst way
//! for a search to fail. Entries are therefore namespaced by the model that
//! produced them, and a client asking under a name the store has never seen
//! gets an empty answer rather than somebody else's geometry.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use brain_core::semantic::Digest;
use serde::{Deserialize, Serialize};

pub mod http;

/// One note's vectors: a unit vector per chunk, in the order the chunks appear.
pub type Chunks = Vec<Vec<f32>>;

/// What a client asks for.
#[derive(Debug, Deserialize, Serialize)]
pub struct FetchRequest {
    /// The model these vectors must have come from.
    pub model: String,
    pub digests: Vec<u64>,
}

/// What comes back. Digests the store does not hold are simply absent — there
/// is no way for one lookup to fail while another succeeds.
#[derive(Debug, Deserialize, Serialize)]
pub struct FetchResponse {
    pub found: Vec<(u64, Chunks)>,
}

/// Vectors a client computed and is offering to everyone else.
#[derive(Debug, Deserialize, Serialize)]
pub struct PublishRequest {
    pub model: String,
    pub entries: Vec<(u64, Chunks)>,
}

/// The vectors, per model, and where they are kept.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    /// Model name to digest to chunks. A `BTreeMap` so the file is stable
    /// between writes and a diff of two dumps means something.
    by_model: BTreeMap<String, BTreeMap<u64, Chunks>>,
}

impl Store {
    /// Read the store, or start an empty one.
    ///
    /// A file that will not parse is treated as absent rather than fatal: this
    /// is a cache, and refusing to start because of one is refusing to serve
    /// something that can be rebuilt by asking the clients to embed again.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Write the store out atomically: temporary file, then rename. A crash
    /// mid-write leaves the previous store rather than half of this one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let text = serde_json::to_string(self)?;
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)
    }

    pub fn len(&self) -> usize {
        self.by_model.values().map(BTreeMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whichever of `digests` this store holds for `model`.
    pub fn fetch(&self, model: &str, digests: &[u64]) -> Vec<(u64, Chunks)> {
        let Some(held) = self.by_model.get(model) else {
            return Vec::new();
        };
        digests
            .iter()
            .filter_map(|digest| held.get(digest).map(|chunks| (*digest, chunks.clone())))
            .collect()
    }

    /// Take on what a client offered. Returns how many were new, so the log
    /// says something useful about whether anyone is actually sharing.
    ///
    /// An entry that is already held is not overwritten: the same digest means
    /// the same text through the same model, so the vectors are the same, and
    /// the cheapest correct thing is to leave them alone.
    pub fn publish(&mut self, model: &str, entries: Vec<(u64, Chunks)>) -> usize {
        let held = self.by_model.entry(model.to_string()).or_default();
        let mut added = 0;
        for (digest, chunks) in entries {
            if chunks.is_empty() {
                continue;
            }
            if held.insert(digest, chunks).is_none() {
                added += 1;
            }
        }
        added
    }
}

/// Answer one request.
///
/// Split out from the listener so the routing is testable without a socket:
/// everything here is a pure function of the request and the store.
///
/// `/health` is deliberately unauthenticated — a container health check should
/// not need the shared secret, and the only thing it discloses is that the
/// service is up and roughly how much it holds.
pub fn route(request: &http::Request, token: &str, store: &mut Store) -> (u16, Vec<u8>) {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => (
            200,
            format!("{{\"ok\":true,\"vectors\":{}}}", store.len()).into_bytes(),
        ),
        (_, "/fetch") | (_, "/publish") if !authorised(request, token) => {
            (401, b"{\"error\":\"unauthorised\"}".to_vec())
        }
        ("POST", "/fetch") => match serde_json::from_slice::<FetchRequest>(&request.body) {
            Ok(asked) => {
                let found = store.fetch(&asked.model, &asked.digests);
                let body = serde_json::to_vec(&FetchResponse { found })
                    .unwrap_or_else(|_| b"{\"found\":[]}".to_vec());
                (200, body)
            }
            Err(_) => (400, b"{\"error\":\"bad request\"}".to_vec()),
        },
        ("POST", "/publish") => match serde_json::from_slice::<PublishRequest>(&request.body) {
            Ok(offered) => {
                let added = store.publish(&offered.model, offered.entries);
                (200, format!("{{\"added\":{added}}}").into_bytes())
            }
            Err(_) => (400, b"{\"error\":\"bad request\"}".to_vec()),
        },
        _ => (404, b"{\"error\":\"no such route\"}".to_vec()),
    }
}

fn authorised(request: &http::Request, token: &str) -> bool {
    request
        .bearer()
        .is_some_and(|given| constant_eq(given, token))
}

/// Compare without returning early on the first differing byte.
///
/// The service sits on a private network and the odds of anyone being in a
/// position to time it are slim, but a shared secret compared with `==` leaks
/// its prefix and the fix is four lines.
fn constant_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Where the store lives by default, inside the container's data volume.
pub fn default_store_path() -> PathBuf {
    std::env::var_os("BRAIN_VECTORS_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/brain-vectors"))
        .join("vectors.json")
}

/// Convert a core digest to the wire representation and back.
///
/// The wire carries the bare `u64` rather than the newtype so the format does
/// not depend on how `Digest` happens to serialise.
pub fn wire(digest: Digest) -> u64 {
    digest.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(value: f32) -> Chunks {
        vec![vec![value, value + 1.0]]
    }

    const TOKEN: &str = "a-token-of-at-least-thirty-two-chars";

    fn request(method: &str, path: &str, token: Option<&str>, body: &str) -> http::Request {
        let auth = match token {
            Some(token) => format!("Authorization: Bearer {token}\r\n"),
            None => String::new(),
        };
        let raw = format!(
            "{method} {path} HTTP/1.1\r\n{auth}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        http::read_request(&mut std::io::BufReader::new(raw.as_bytes())).expect("parse")
    }

    #[test]
    fn health_needs_no_token() {
        // A container health check should not need the shared secret, and all
        // it discloses is that the service is up.
        let mut store = Store::default();
        let (status, body) = route(&request("GET", "/health", None, ""), TOKEN, &mut store);

        assert_eq!(status, 200);
        assert_eq!(
            String::from_utf8_lossy(&body),
            "{\"ok\":true,\"vectors\":0}"
        );
    }

    #[test]
    fn fetching_without_a_token_is_refused() {
        let mut store = Store::default();
        let asked = "{\"model\":\"nomic\",\"digests\":[7]}";

        let (status, _) = route(&request("POST", "/fetch", None, asked), TOKEN, &mut store);
        assert_eq!(status, 401);

        let (status, _) = route(
            &request("POST", "/fetch", Some("wrong"), asked),
            TOKEN,
            &mut store,
        );
        assert_eq!(status, 401);
    }

    #[test]
    fn a_publish_then_a_fetch_round_trips_over_the_wire_format() {
        let mut store = Store::default();

        let (status, body) = route(
            &request(
                "POST",
                "/publish",
                Some(TOKEN),
                "{\"model\":\"nomic\",\"entries\":[[7,[[0.5,0.5]]]]}",
            ),
            TOKEN,
            &mut store,
        );
        assert_eq!(status, 200);
        assert_eq!(String::from_utf8_lossy(&body), "{\"added\":1}");

        let (status, body) = route(
            &request(
                "POST",
                "/fetch",
                Some(TOKEN),
                "{\"model\":\"nomic\",\"digests\":[7,9]}",
            ),
            TOKEN,
            &mut store,
        );
        assert_eq!(status, 200);
        let found: FetchResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(found.found, vec![(7, vec![vec![0.5, 0.5]])]);
    }

    #[test]
    fn a_body_that_is_not_the_expected_shape_is_a_bad_request() {
        let mut store = Store::default();
        let (status, _) = route(
            &request("POST", "/fetch", Some(TOKEN), "{\"model\":\"nomic\"}"),
            TOKEN,
            &mut store,
        );

        assert_eq!(status, 400);
    }

    #[test]
    fn an_unknown_route_is_a_404_and_not_a_hint() {
        let mut store = Store::default();
        let (status, _) = route(
            &request("GET", "/vectors", Some(TOKEN), ""),
            TOKEN,
            &mut store,
        );

        assert_eq!(status, 404);
    }

    #[test]
    fn a_token_of_the_wrong_length_is_refused_without_comparing_it() {
        assert!(!constant_eq("short", TOKEN));
        assert!(constant_eq(TOKEN, TOKEN));
    }

    #[test]
    fn a_published_vector_comes_back() {
        let mut store = Store::default();
        store.publish("nomic", vec![(7, chunks(1.0))]);

        assert_eq!(store.fetch("nomic", &[7]), vec![(7, chunks(1.0))]);
    }

    #[test]
    fn a_digest_the_store_does_not_hold_is_absent_rather_than_an_error() {
        let mut store = Store::default();
        store.publish("nomic", vec![(7, chunks(1.0))]);

        assert_eq!(store.fetch("nomic", &[7, 8]), vec![(7, chunks(1.0))]);
    }

    #[test]
    fn another_model_never_sees_these_vectors() {
        let mut store = Store::default();
        store.publish("nomic", vec![(7, chunks(1.0))]);

        // Vectors from two models rank plausibly and wrongly, which is worse
        // than returning nothing and letting the client embed.
        assert!(store.fetch("e5", &[7]).is_empty());
    }

    #[test]
    fn republishing_the_same_digest_is_not_counted_as_new() {
        let mut store = Store::default();

        assert_eq!(store.publish("nomic", vec![(7, chunks(1.0))]), 1);
        assert_eq!(store.publish("nomic", vec![(7, chunks(1.0))]), 0);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn an_empty_vector_is_refused_rather_than_cached() {
        let mut store = Store::default();

        // A note that produced no chunks has nothing worth sharing, and
        // caching the emptiness would stop the next machine from trying.
        assert_eq!(store.publish("nomic", vec![(7, Vec::new())]), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn a_store_survives_a_round_trip_through_the_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("nested/vectors.json");
        let mut store = Store::default();
        store.publish("nomic", vec![(7, chunks(1.0)), (8, chunks(2.0))]);

        store.save(&path).expect("save");
        let read = Store::load(&path);

        assert_eq!(read.len(), 2);
        assert_eq!(read.fetch("nomic", &[8]), vec![(8, chunks(2.0))]);
    }

    #[test]
    fn a_corrupt_store_starts_empty_rather_than_refusing_to_start() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("vectors.json");
        std::fs::write(&path, "{ this is not json").expect("write");

        // It is a cache. Refusing to serve because of one is worse than asking
        // the clients to embed again.
        assert!(Store::load(&path).is_empty());
    }
}
