//! The client for `brain-vectors`: vectors other machines already paid for.
//!
//! The second socket Brain opens, and the second thing that is only ever
//! spoken to from a worker thread. It implements [`semantic::Shared`], which
//! is the seam `DESIGN.md` named — "if these notes ever need to be searched
//! from another machine, it is one type to reimplement".
//!
//! **Note text never comes here.** Only digests go out and vectors come back,
//! so a store on a machine you do not control learns which *fingerprints* a
//! vault holds and nothing about what the notes say. That is a deliberately
//! smaller disclosure than the embedding server itself gets, and it is why
//! this could reasonably live somewhere the model server should not.
//!
//! Every failure is swallowed into "no shared store this pass". There is
//! nothing else to do: the vectors are derived, and a store that cannot be
//! reached costs one pass of embedding.

use brain_core::semantic::{Chunks, Digest, EmbedError, Shared};
use serde::{Deserialize, Serialize};

/// The port `brain-vectors` listens on, matching its Containerfile.
pub const DEFAULT_VECTORS_PORT: u16 = 8082;

#[derive(Serialize)]
struct FetchRequest<'a> {
    model: &'a str,
    digests: Vec<u64>,
}

#[derive(Deserialize)]
struct FetchResponse {
    found: Vec<(u64, Chunks)>,
}

#[derive(Serialize)]
struct PublishRequest<'a> {
    model: &'a str,
    entries: Vec<(u64, Chunks)>,
}

/// A `brain-vectors` service, over HTTP.
///
/// Owns its own `soup::Session`, because sessions belong to the thread that
/// made them and this only ever runs on the catch-up worker.
pub struct Service {
    session: soup::Session,
    url: String,
    token: String,
    model: String,
}

impl Service {
    /// Point at a service. Nothing is contacted until a pass runs — a store
    /// that is down should not stop the app from starting.
    pub fn new(url: &str, token: &str, model: &str) -> Self {
        Self {
            session: soup::Session::new(),
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            model: model.to_string(),
        }
    }

    fn post(&self, route: &str, body: &str) -> Result<String, EmbedError> {
        use soup::prelude::*;

        let url = format!("{}{route}", self.url);
        let message = soup::Message::new("POST", &url)
            .map_err(|_| EmbedError(format!("{url} is not a URL")))?;
        if let Some(headers) = message.request_headers() {
            headers.append("Authorization", &format!("Bearer {}", self.token));
        }
        message.set_request_body_from_bytes(
            Some("application/json"),
            Some(&gtk::glib::Bytes::from_owned(body.as_bytes().to_vec())),
        );

        let bytes = self
            .session
            .send_and_read(&message, gtk::gio::Cancellable::NONE)
            .map_err(|error| EmbedError(format!("no answer from the vector store: {error}")))?;
        let status = message.status_code();
        if !(200..300).contains(&status) {
            // 401 is the one worth reading twice: it means the token is wrong,
            // and every pass will keep quietly embedding everything locally
            // until someone notices.
            return Err(EmbedError(format!("the vector store said {status}")));
        }
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }
}

impl Shared for Service {
    fn fetch(&self, digests: &[Digest]) -> Result<Vec<(Digest, Chunks)>, EmbedError> {
        if digests.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::to_string(&FetchRequest {
            model: &self.model,
            digests: digests.iter().map(|digest| digest.0).collect(),
        })
        .map_err(|error| EmbedError(error.to_string()))?;

        let text = self.post("/fetch", &body)?;
        let answer: FetchResponse =
            serde_json::from_str(&text).map_err(|error| EmbedError(error.to_string()))?;
        Ok(answer
            .found
            .into_iter()
            .map(|(digest, chunks)| (Digest(digest), chunks))
            .collect())
    }

    fn publish(&self, entries: &[(Digest, Chunks)]) -> Result<(), EmbedError> {
        if entries.is_empty() {
            return Ok(());
        }
        let body = serde_json::to_string(&PublishRequest {
            model: &self.model,
            entries: entries
                .iter()
                .map(|(digest, chunks)| (digest.0, chunks.clone()))
                .collect(),
        })
        .map_err(|error| EmbedError(error.to_string()))?;

        self.post("/publish", &body).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_does_not_become_a_double_one() {
        // `http://nas:8082/` and `http://nas:8082` name the same service, and
        // `//fetch` is a 404 on one of them.
        let service = Service::new("http://nas:8082/", "token", "nomic");
        assert_eq!(service.url, "http://nas:8082");
    }

    #[test]
    fn the_bodies_are_byte_for_byte_what_the_service_accepts() {
        // The client and the server define this shape in two crates that never
        // see each other's types, so it is pinned on both sides. These exact
        // strings are what `brain-vectors`' own round-trip test parses.
        let fetch = serde_json::to_string(&FetchRequest {
            model: "nomic",
            digests: vec![7, 9],
        })
        .expect("serialise");
        assert_eq!(fetch, r#"{"model":"nomic","digests":[7,9]}"#);

        let publish = serde_json::to_string(&PublishRequest {
            model: "nomic",
            entries: vec![(7, vec![vec![0.5, 0.5]])],
        })
        .expect("serialise");
        assert_eq!(publish, r#"{"model":"nomic","entries":[[7,[[0.5,0.5]]]]}"#);

        let answer: FetchResponse =
            serde_json::from_str(r#"{"found":[[7,[[0.5,0.5]]]]}"#).expect("parse");
        assert_eq!(answer.found, vec![(7, vec![vec![0.5, 0.5]])]);
    }

    #[test]
    fn asking_about_nothing_does_not_open_a_connection() {
        // A quiet vault runs a pass every few seconds and has nothing to say.
        // Waking a socket to tell the store so is work for no reason, and it
        // means an unconfigured store logs nothing rather than a failure a
        // second.
        let service = Service::new("http://127.0.0.1:1", "token", "nomic");
        assert_eq!(service.fetch(&[]).expect("no request"), Vec::new());
        assert!(service.publish(&[]).is_ok());
    }
}
