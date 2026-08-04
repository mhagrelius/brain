//! The client for `brain-server`'s vault routes.
//!
//! [`sync::Remote`]'s implementation, and the third socket Brain opens. Same
//! arrangement as [`crate::ui::embedder`] and [`crate::ui::shared_vectors`]:
//! its own `soup::Session`, made and dropped on the worker thread, blocking
//! calls, and every failure turned into "not this pass".
//!
//! Unlike the vector store, **this one gets the notes themselves**. That is
//! unavoidable — it is holding the vault — and it is the reason a URL and a
//! token have to be typed in on purpose rather than defaulted to anything.

use brain_core::sync::{Hash, Put, Remote, Snapshot, SyncError};
use serde::{Deserialize, Serialize};

use crate::model::note::NoteId;

#[derive(Deserialize)]
struct Listed {
    id: String,
    hash: u64,
}

#[derive(Deserialize)]
struct ListResponse {
    notes: Vec<Listed>,
}

#[derive(Serialize)]
struct GetRequest {
    ids: Vec<String>,
}

#[derive(Deserialize)]
struct Fetched {
    id: String,
    text: String,
}

#[derive(Deserialize)]
struct GetResponse {
    notes: Vec<Fetched>,
}

#[derive(Serialize)]
struct PutRequest<'a> {
    id: &'a str,
    text: &'a str,
    base: Option<u64>,
}

#[derive(Serialize)]
struct DeleteRequest<'a> {
    id: &'a str,
    base: Option<u64>,
}

#[derive(Deserialize)]
struct Stale {
    current: Option<u64>,
}

/// A `brain-server` vault, over HTTP.
pub struct Service {
    session: soup::Session,
    url: String,
    token: String,
}

impl Service {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            session: soup::Session::new(),
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// Send one request and give back the status and the body.
    ///
    /// The status comes back rather than being folded into an error, because
    /// 409 is not a failure — it is the server saying what it holds, which is
    /// exactly what a write needs to hear.
    fn send(&self, method: &str, route: &str, body: Option<&str>) -> Result<(u32, String), String> {
        use soup::prelude::*;

        let url = format!("{}{route}", self.url);
        let message =
            soup::Message::new(method, &url).map_err(|_| format!("{url} is not a URL"))?;
        if let Some(headers) = message.request_headers() {
            headers.append("Authorization", &format!("Bearer {}", self.token));
        }
        if let Some(body) = body {
            message.set_request_body_from_bytes(
                Some("application/json"),
                Some(&gtk::glib::Bytes::from_owned(body.as_bytes().to_vec())),
            );
        }
        let bytes = self
            .session
            .send_and_read(&message, gtk::gio::Cancellable::NONE)
            .map_err(|error| format!("no answer from the vault server: {error}"))?;
        Ok((
            message.status_code(),
            String::from_utf8_lossy(&bytes).to_string(),
        ))
    }

    fn ok(&self, method: &str, route: &str, body: Option<&str>) -> Result<String, SyncError> {
        let (status, text) = self.send(method, route, body).map_err(SyncError)?;
        if !(200..300).contains(&status) {
            return Err(SyncError(format!("the vault server said {status}")));
        }
        Ok(text)
    }

    /// The shared tail of put and delete: both answer the same three ways.
    fn wrote(&self, route: &str, body: &str) -> Put {
        match self.send("POST", route, Some(body)) {
            Err(error) => Put::Failed(error),
            Ok((status, text)) if (200..300).contains(&status) => {
                match serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|parsed| parsed["hash"].as_u64())
                {
                    Some(hash) => Put::Done(Hash(hash)),
                    None => Put::Failed("the vault server sent no hash back".into()),
                }
            }
            Ok((409, text)) => match serde_json::from_str::<Stale>(&text) {
                Ok(stale) => Put::Stale(stale.current.map(Hash)),
                // A refusal that cannot be read is still a refusal, and
                // treating it as success would overwrite somebody's note.
                Err(_) => Put::Failed("the vault server refused, unreadably".into()),
            },
            Ok((status, _)) => Put::Failed(format!("the vault server said {status}")),
        }
    }
}

impl Remote for Service {
    fn list(&self) -> Result<Snapshot, SyncError> {
        let text = self.ok("GET", "/notes", None)?;
        let listed: ListResponse =
            serde_json::from_str(&text).map_err(|error| SyncError(error.to_string()))?;
        Ok(listed
            .notes
            .into_iter()
            .map(|note| (NoteId::from_relative(note.id), Hash(note.hash)))
            .collect())
    }

    fn get(&self, ids: &[NoteId]) -> Result<Vec<(NoteId, String)>, SyncError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::to_string(&GetRequest {
            ids: ids.iter().map(|id| id.as_str().to_string()).collect(),
        })
        .map_err(|error| SyncError(error.to_string()))?;

        let text = self.ok("POST", "/notes/get", Some(&body))?;
        let got: GetResponse =
            serde_json::from_str(&text).map_err(|error| SyncError(error.to_string()))?;
        Ok(got
            .notes
            .into_iter()
            .map(|note| (NoteId::from_relative(note.id), note.text))
            .collect())
    }

    fn put(&self, id: &NoteId, text: &str, base: Option<Hash>) -> Put {
        let Ok(body) = serde_json::to_string(&PutRequest {
            id: id.as_str(),
            text,
            base: base.map(|hash| hash.0),
        }) else {
            return Put::Failed("could not encode the note".into());
        };
        self.wrote("/notes/put", &body)
    }

    fn delete(&self, id: &NoteId, base: Option<Hash>) -> Put {
        let Ok(body) = serde_json::to_string(&DeleteRequest {
            id: id.as_str(),
            base: base.map(|hash| hash.0),
        }) else {
            return Put::Failed("could not encode the id".into());
        };
        self.wrote("/notes/delete", &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bodies_are_byte_for_byte_what_the_service_accepts() {
        // Pinned on both sides: `brain-server`'s own tests parse these exact
        // strings. The two crates never see each other's types, so this is the
        // only thing that notices if one of them drifts.
        assert_eq!(
            serde_json::to_string(&GetRequest {
                ids: vec!["A.md".into()]
            })
            .expect("encode"),
            r#"{"ids":["A.md"]}"#
        );
        assert_eq!(
            serde_json::to_string(&PutRequest {
                id: "A.md",
                text: "body",
                base: Some(7),
            })
            .expect("encode"),
            r#"{"id":"A.md","text":"body","base":7}"#
        );
        assert_eq!(
            serde_json::to_string(&DeleteRequest {
                id: "A.md",
                base: None,
            })
            .expect("encode"),
            r#"{"id":"A.md","base":null}"#
        );

        let listed: ListResponse =
            serde_json::from_str(r#"{"notes":[{"id":"A.md","hash":7}]}"#).expect("parse");
        assert_eq!(listed.notes[0].id, "A.md");
        assert_eq!(listed.notes[0].hash, 7);

        let got: GetResponse =
            serde_json::from_str(r#"{"notes":[{"id":"A.md","text":"body","hash":7}]}"#)
                .expect("parse");
        assert_eq!(got.notes[0].text, "body");

        let stale: Stale = serde_json::from_str(r#"{"current":9}"#).expect("parse");
        assert_eq!(stale.current, Some(9));
        let deleted: Stale = serde_json::from_str(r#"{"current":null}"#).expect("parse");
        assert_eq!(deleted.current, None);
    }

    #[test]
    fn asking_for_no_notes_does_not_open_a_connection() {
        let service = Service::new("http://127.0.0.1:1", "token");
        assert!(service.get(&[]).expect("no request").is_empty());
    }
}
