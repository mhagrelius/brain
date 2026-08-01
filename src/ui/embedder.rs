//! Vectors from a llama.cpp server.
//!
//! The only socket Brain opens. It talks to `llama-server` started with
//! `--embeddings`, over the OpenAI-shaped `/v1/embeddings` route, on the
//! loopback address unless told otherwise.
//!
//! **Note text goes to whatever this is pointed at**, which is the whole reason
//! it is a plain URL and defaults to a port on this machine: a note is the most
//! private thing a person owns, and where its text is allowed to go should be a
//! decision, taken once, in a config file — not a property of a hosted service
//! that changed under you. A box on your own network is a reasonable answer and
//! a NAS over Tailscale is the one this was built against; an endpoint on the
//! open internet would work too, and is exactly what nobody should reach for
//! without meaning it.
//!
//! # Why this is blocking, on a thread
//!
//! Everything else in Brain runs on the main loop. This does not, because a
//! catch-up pass is a batch: on a first run it is every note in the vault, one
//! request each, and there is nothing for the user to watch while it happens.
//! Running it on a worker thread with the synchronous API keeps
//! [`crate::model::semantic::catch_up`] a plain function — which is what lets
//! the interesting behaviour be tested against a fake instead of a GPU.
//!
//! The thread owns its own `soup::Session`. Sessions belong to the thread that
//! made them, and sharing one would be the kind of bug that only shows up on a
//! slow day.

use crate::model::semantic::{EmbedError, Embedder};

/// Where the embedding server is, if none is configured.
///
/// Port 8081 rather than 8080: 8080 is where the chat model already lives on
/// this kind of setup, and an embedding model is a second, much smaller server
/// beside it rather than a replacement for it.
pub const DEFAULT_EMBEDDING_URL: &str = "http://127.0.0.1:8081";

/// What a model wants prepended to a passage and to a question.
///
/// The retrieval models worth running locally are trained asymmetrically, and
/// the prefix is not decoration: it is how the model is told which side of the
/// asymmetry this text is on. Omitting it costs recall silently, since the
/// vectors still come back and still rank — measurably worse, with nothing to
/// show for it.
///
/// Matched on the model's name because that is all the server offers. An
/// unrecognised model gets no prefixes, which is right for the models that want
/// none and harmless for one whose convention we have not heard of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefixes {
    /// Names this scheme, so the store can invalidate when it changes. Vectors
    /// made with a prefix and without it are not comparable, and the model's
    /// own name would not have changed to say so.
    pub scheme: &'static str,
    pub document: &'static str,
    pub query: &'static str,
}

const NONE: Prefixes = Prefixes {
    scheme: "plain",
    document: "",
    query: "",
};

pub fn prefixes_for(model: &str) -> Prefixes {
    let name = model.to_lowercase();
    if name.contains("nomic-embed") {
        Prefixes {
            scheme: "nomic",
            document: "search_document: ",
            query: "search_query: ",
        }
    } else if name.contains("e5") {
        Prefixes {
            scheme: "e5",
            document: "passage: ",
            query: "query: ",
        }
    } else if name.contains("bge") && !name.contains("m3") {
        // BGE prefixes the question only, and says so in its model card.
        Prefixes {
            scheme: "bge",
            document: "",
            query: "Represent this sentence for searching relevant passages: ",
        }
    } else {
        NONE
    }
}

/// A llama.cpp server that turns text into vectors.
pub struct Llama {
    base: String,
    session: soup::Session,
    model: String,
    prefixes: Prefixes,
}

impl Llama {
    /// Connect and ask the server what it is serving.
    ///
    /// The model's name comes from the server rather than from configuration
    /// because it is what the store keys on: pointing Brain at a different
    /// model has to invalidate the vectors, and a name the user typed could
    /// agree while the model underneath had changed.
    ///
    /// Called on the worker thread. Failing here is ordinary — it means no
    /// server is running — and the caller's answer is to search lexically.
    pub fn connect(base: &str) -> Result<Self, EmbedError> {
        let session = soup::Session::new();
        let base = base.trim_end_matches('/').to_string();
        let body = get(&session, &format!("{base}/v1/models"))?;

        let parsed: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| EmbedError(format!("the server's model list is not JSON: {error}")))?;
        let model = parsed["data"][0]["id"]
            .as_str()
            .ok_or_else(|| EmbedError("the server named no model".into()))?
            .to_string();

        let prefixes = prefixes_for(&model);
        Ok(Self {
            base,
            session,
            model,
            prefixes,
        })
    }

    fn embed_prefixed(&self, prefix: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let input: Vec<String> = texts.iter().map(|text| format!("{prefix}{text}")).collect();
        let request = serde_json::json!({ "input": input, "model": self.model });
        let body = post(
            &self.session,
            &format!("{}/v1/embeddings", self.base),
            &request.to_string(),
        )?;
        parse_embeddings(&body, texts.len())
    }
}

impl Embedder for Llama {
    /// The model *and* how it was prompted.
    ///
    /// Changing the prefix scheme changes every vector the model produces, and
    /// the model's own name would not move to say so — the store would go on
    /// comparing new vectors against old ones and rank plausibly and wrongly.
    fn model(&self) -> String {
        format!("{}+{}", self.model, self.prefixes.scheme)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.embed_prefixed(self.prefixes.document, texts)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed_prefixed(
            self.prefixes.query,
            std::slice::from_ref(&query.to_string()),
        )?
        .into_iter()
        .next()
        .ok_or_else(|| EmbedError("no vector came back for the query".into()))
    }
}

/// Pull the vectors out of a `/v1/embeddings` response.
///
/// Separate from the request so it can be tested against a recorded body — the
/// shape of this reply is the one thing here that can change under Brain
/// without anything failing loudly.
///
/// The server returns the vectors with an `index` each, and they are sorted by
/// it rather than trusted to arrive in order: a batch silently reordered would
/// attach every note's vector to the note beside it, which no assertion in the
/// app would ever catch.
pub fn parse_embeddings(body: &str, expected: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let parsed: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| EmbedError(format!("the reply is not JSON: {error}")))?;

    if let Some(message) = parsed["error"]["message"].as_str() {
        return Err(EmbedError(message.to_string()));
    }
    let data = parsed["data"]
        .as_array()
        .ok_or_else(|| EmbedError("the reply carried no data".into()))?;

    let mut vectors: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
    for (position, entry) in data.iter().enumerate() {
        // llama.cpp answers a pooled request with `embedding: [..]` and an
        // unpooled one with `embedding: [[..]]`. Take the first row either way,
        // rather than silently embedding a vault against a rank-2 reply.
        let values = entry["embedding"]
            .as_array()
            .and_then(|array| match array.first() {
                Some(serde_json::Value::Array(_)) => array.first()?.as_array(),
                _ => Some(array),
            })
            .ok_or_else(|| EmbedError("an entry carried no embedding".into()))?;
        let vector: Vec<f32> = values
            .iter()
            .filter_map(|value| value.as_f64().map(|v| v as f32))
            .collect();
        if vector.len() != values.len() {
            return Err(EmbedError(
                "an embedding held something that is not a number".into(),
            ));
        }
        let index = entry["index"]
            .as_u64()
            .map(|i| i as usize)
            .unwrap_or(position);
        vectors.push((index, vector));
    }

    if vectors.len() != expected {
        return Err(EmbedError(format!(
            "asked for {expected} embeddings and got {}",
            vectors.len()
        )));
    }
    vectors.sort_by_key(|(index, _)| *index);
    Ok(vectors.into_iter().map(|(_, vector)| vector).collect())
}

fn get(session: &soup::Session, url: &str) -> Result<String, EmbedError> {
    let message =
        soup::Message::new("GET", url).map_err(|_| EmbedError(format!("{url} is not a URL")))?;
    send(session, &message)
}

fn post(session: &soup::Session, url: &str, body: &str) -> Result<String, EmbedError> {
    let message =
        soup::Message::new("POST", url).map_err(|_| EmbedError(format!("{url} is not a URL")))?;
    message.set_request_body_from_bytes(
        Some("application/json"),
        Some(&gtk::glib::Bytes::from_owned(body.as_bytes().to_vec())),
    );
    send(session, &message)
}

fn send(session: &soup::Session, message: &soup::Message) -> Result<String, EmbedError> {
    use soup::prelude::*;

    let bytes = session
        .send_and_read(message, gtk::gio::Cancellable::NONE)
        .map_err(|error| EmbedError(format!("no answer from the model server: {error}")))?;
    let status = message.status_code();
    if !(200..300).contains(&status) {
        // The body carries llama.cpp's own explanation — "start it with
        // --embeddings" is the one people actually hit — so it is worth
        // keeping rather than reporting the number alone.
        let text = String::from_utf8_lossy(&bytes);
        let detail = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|parsed| parsed["error"]["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(EmbedError(format!(
            "the model server said {status}: {detail}"
        )));
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_gets_the_prefixes_its_card_asks_for() {
        assert_eq!(
            prefixes_for("nomic-embed-text-v1.5").query,
            "search_query: "
        );
        assert_eq!(
            prefixes_for("nomic-embed-text-v1.5").document,
            "search_document: "
        );
        assert_eq!(prefixes_for("multilingual-e5-large").document, "passage: ");
        // BGE prefixes the question and leaves the passage alone.
        assert_eq!(prefixes_for("bge-small-en-v1.5").document, "");
        assert!(prefixes_for("bge-small-en-v1.5")
            .query
            .starts_with("Represent"));
        // Anything unheard of is left alone rather than guessed at.
        assert_eq!(prefixes_for("some-new-model").scheme, "plain");
        assert_eq!(prefixes_for("some-new-model").query, "");
    }

    #[test]
    fn the_scheme_is_part_of_what_the_store_keys_on() {
        // Two schemes over the same model produce vectors that are not
        // comparable, and the model's name alone would not say so.
        assert_ne!(
            prefixes_for("nomic-embed-text-v1.5").scheme,
            prefixes_for("bge-small-en-v1.5").scheme
        );
    }

    #[test]
    fn a_pooled_reply_parses() {
        let body = r#"{"data":[{"embedding":[0.1,0.2],"index":0},
                               {"embedding":[0.3,0.4],"index":1}]}"#;
        assert_eq!(
            parse_embeddings(body, 2).expect("parse"),
            [[0.1, 0.2], [0.3, 0.4]]
        );
    }

    #[test]
    fn an_unpooled_reply_parses_too() {
        // llama.cpp wraps the vector in another array when pooling is off.
        let body = r#"{"data":[{"embedding":[[0.1,0.2]],"index":0}]}"#;
        assert_eq!(parse_embeddings(body, 1).expect("parse"), [[0.1, 0.2]]);
    }

    #[test]
    fn vectors_are_put_back_in_the_order_they_were_asked_for() {
        // A batch quietly reordered would give every note the vector of the
        // note beside it, and nothing downstream could tell.
        let body = r#"{"data":[{"embedding":[9.0],"index":1},
                               {"embedding":[1.0],"index":0}]}"#;
        assert_eq!(parse_embeddings(body, 2).expect("parse"), [[1.0], [9.0]]);
    }

    #[test]
    fn a_server_without_embeddings_turned_on_says_so() {
        let body = r#"{"error":{"code":501,
                                "message":"This server does not support embeddings. Start it with `--embeddings`",
                                "type":"not_supported_error"}}"#;
        let error = parse_embeddings(body, 1).expect_err("should fail");
        assert!(error.0.contains("--embeddings"), "{error}");
    }

    #[test]
    fn a_short_batch_is_a_failure_rather_than_a_misalignment() {
        let body = r#"{"data":[{"embedding":[0.1],"index":0}]}"#;
        let error = parse_embeddings(body, 3).expect_err("should fail");
        assert!(error.0.contains("asked for 3"), "{error}");
    }

    #[test]
    fn nonsense_is_an_error_and_not_a_panic() {
        for body in ["", "not json", "{}", r#"{"data":[{}]}"#] {
            assert!(parse_embeddings(body, 1).is_err(), "{body:?} parsed");
        }
    }
}
