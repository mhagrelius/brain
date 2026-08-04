//! Just enough HTTP/1.1 to serve three routes to one client.
//!
//! Request line, headers to the blank line, `Content-Length`, body, respond,
//! close. No keep-alive, no chunked transfer, no TLS — a reverse proxy or a
//! Tailscale address is where those belong, and pretending otherwise here
//! would be a worse version of something that already exists.
//!
//! Everything parses from a `BufRead`, so the tests are byte slices rather
//! than sockets.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

/// The largest request head this will read, and the largest body.
///
/// A publish of a whole vault is a few megabytes of floats; ten leaves room
/// without letting a stray connection ask for a gigabyte of memory.
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 10 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Lowercased names, so a lookup does not depend on how the client cased
    /// them. HTTP says they are insensitive and clients take that literally.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// The bearer token, if the request carried one.
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")?.strip_prefix("Bearer ")
    }
}

/// Why a request could not be understood. Each maps to a status, and none of
/// them says anything about *why* to the caller — a parser that explains
/// itself to an unauthenticated stranger is a parser that helps them.
#[derive(Debug, PartialEq, Eq)]
pub enum Bad {
    Malformed,
    TooLarge,
    /// The connection closed before a request arrived. Not an error: a health
    /// checker that opens a socket and hangs up does this every few seconds.
    Closed,
}

pub fn read_request(input: &mut impl BufRead) -> Result<Request, Bad> {
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        // `read_until` stops at the newline and keeps it, so a bare LF and a
        // CRLF both terminate — some clients send the first.
        match input.read_until(b'\n', &mut line) {
            Ok(0) if head.is_empty() => return Err(Bad::Closed),
            Ok(0) => return Err(Bad::Malformed),
            Ok(_) => {}
            Err(_) => return Err(Bad::Malformed),
        }
        let blank = line == b"\r\n" || line == b"\n";
        head.extend_from_slice(&line);
        if head.len() > MAX_HEAD {
            return Err(Bad::TooLarge);
        }
        if blank {
            break;
        }
    }

    let text = String::from_utf8(head).map_err(|_| Bad::Malformed)?;
    let mut lines = text.lines();
    let mut start = lines.next().ok_or(Bad::Malformed)?.split_whitespace();
    let method = start.next().ok_or(Bad::Malformed)?.to_string();
    let path = start.next().ok_or(Bad::Malformed)?.to_string();

    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or(Bad::Malformed)?;
        headers.insert(name.trim().to_lowercase(), value.trim().to_string());
    }

    let length: usize = match headers.get("content-length") {
        Some(value) => value.trim().parse().map_err(|_| Bad::Malformed)?,
        None => 0,
    };
    if length > MAX_BODY {
        return Err(Bad::TooLarge);
    }
    let mut body = vec![0u8; length];
    input.read_exact(&mut body).map_err(|_| Bad::Malformed)?;

    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

/// Write a response and be done with the connection.
pub fn respond(output: &mut impl Write, status: u16, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    write!(
        output,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    output.write_all(body)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<Request, Bad> {
        read_request(&mut io::BufReader::new(raw.as_bytes()))
    }

    #[test]
    fn a_get_with_no_body_parses() {
        let request = parse("GET /health HTTP/1.1\r\nHost: nas\r\n\r\n").expect("parse");

        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/health");
        assert_eq!(request.header("host"), Some("nas"));
        assert!(request.body.is_empty());
    }

    #[test]
    fn a_post_body_is_read_to_its_content_length() {
        let request =
            parse("POST /fetch HTTP/1.1\r\nContent-Length: 7\r\n\r\n{\"a\":1}").expect("parse");

        assert_eq!(request.body, b"{\"a\":1}");
    }

    #[test]
    fn header_names_are_matched_whatever_their_case() {
        let request = parse("GET / HTTP/1.1\r\nCoNtEnT-lEnGtH: 0\r\n\r\n").expect("parse");

        assert_eq!(request.header("content-length"), Some("0"));
    }

    #[test]
    fn a_bearer_token_is_pulled_out_of_the_authorization_header() {
        let request =
            parse("GET / HTTP/1.1\r\nAuthorization: Bearer hunter2\r\n\r\n").expect("parse");

        assert_eq!(request.bearer(), Some("hunter2"));
    }

    #[test]
    fn a_non_bearer_authorization_is_not_mistaken_for_one() {
        let request = parse("GET / HTTP/1.1\r\nAuthorization: Basic abc\r\n\r\n").expect("parse");

        assert_eq!(request.bearer(), None);
    }

    #[test]
    fn a_connection_that_closes_without_asking_anything_is_not_an_error() {
        assert_eq!(parse(""), Err(Bad::Closed));
    }

    #[test]
    fn a_body_shorter_than_its_content_length_is_malformed() {
        assert_eq!(
            parse("POST /fetch HTTP/1.1\r\nContent-Length: 99\r\n\r\nshort"),
            Err(Bad::Malformed)
        );
    }

    #[test]
    fn a_content_length_that_is_not_a_number_is_malformed() {
        assert_eq!(
            parse("POST /fetch HTTP/1.1\r\nContent-Length: lots\r\n\r\n"),
            Err(Bad::Malformed)
        );
    }

    #[test]
    fn an_enormous_body_is_refused_before_it_is_allocated() {
        assert_eq!(
            parse("POST /publish HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n"),
            Err(Bad::TooLarge)
        );
    }

    #[test]
    fn an_endless_stream_of_headers_is_refused() {
        let mut raw = String::from("GET / HTTP/1.1\r\n");
        for n in 0..2000 {
            raw.push_str(&format!("X-Filler-{n}: padding padding padding\r\n"));
        }
        raw.push_str("\r\n");

        assert_eq!(parse(&raw), Err(Bad::TooLarge));
    }

    #[test]
    fn a_response_carries_its_length_and_closes() {
        let mut out = Vec::new();
        respond(&mut out, 200, b"{}").expect("write");
        let text = String::from_utf8(out).expect("utf-8");

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
    }
}
