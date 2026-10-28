//! The stdio relay behind `vcmp mcp`, for MCP clients that only start local programs
//! (Claude Desktop). It reads JSON-RPC messages from its input, one per line, posts
//! each to the running VCMP's server, and writes every message of the answer to its
//! output, one per line.
//!
//! VCMP has to be open with its AI agents switch on. When it isn't, or the token no
//! longer matches, a request gets an error answer that says so, rather than silence.
//! Requests are relayed one at a time, in the order they come.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use serde_json::{Value, json};

use crate::address;

/// Where the running VCMP listens, and the token it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub port: u16,
    pub token: String,
}

/// Reads the connection from VCMP's settings: again after a request fails, as the
/// owner may have changed the port or made a new token. The error says why there is
/// none (the switch is off, there is no library yet).
pub type Settings<'a> = dyn Fn() -> Result<Connection, String> + 'a;

/// The longest answer read: a page of 500 tracks with all their tags fits easily.
const ANSWER_LIMIT: u64 = 64 * 1024 * 1024;
/// A tool call waits for the window, which waits 30 seconds at most.
const TIMEOUT: Duration = Duration::from_secs(120);
/// JSON-RPC's code for a server error, as the relay answers for VCMP.
const RELAY_ERROR: i64 = -32000;

/// Relays until the input ends, then ends the session.
pub fn run(settings: &Settings<'_>, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut relay = Relay {
        http: ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into(),
        connection: None,
        session: None,
        protocol: None,
    };
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        for message in relay.forward(&line, settings) {
            writeln!(output, "{message}")?;
            output.flush()?;
        }
    }
    relay.close();
    Ok(())
}

struct Relay {
    http: ureq::Agent,
    connection: Option<Connection>,
    session: Option<String>,
    // The protocol version the handshake agreed, sent with every later request.
    protocol: Option<String>,
}

/// Why a request didn't get through.
enum Failure {
    /// Nothing listens on the port: VCMP is closed, or its server didn't start.
    Unreachable,
    Unauthorized,
    /// The server forgot the session: VCMP was restarted, or the switch turned off
    /// and on.
    SessionGone,
    Status(u16, String),
    Settings(String),
}

/// A request's answer.
struct Answer {
    content_type: String,
    body: String,
    session: Option<String>,
}

impl Relay {
    /// Posts one message and returns the messages to write back.
    fn forward(&mut self, line: &str, settings: &Settings<'_>) -> Vec<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                return vec![error_answer(&Value::Null, &format!("not JSON: {error}"))];
            }
        };
        let id = message.get("id").cloned();
        let initialize = message.get("method").and_then(Value::as_str) == Some("initialize");
        if initialize {
            self.session = None;
            self.protocol = None;
        }
        let answer = match self.post(line, settings) {
            Ok(answer) => answer,
            Err(failure) => {
                let reason = reason(&failure);
                log::warn!("not relayed: {reason}");
                return match id {
                    Some(id) => vec![error_answer(&id, &reason)],
                    // A notification has no answer to carry the reason.
                    None => Vec::new(),
                };
            }
        };
        if initialize {
            self.session = answer.session.clone();
        }
        let messages = messages(&answer.content_type, &answer.body);
        if initialize {
            self.protocol = messages
                .iter()
                .filter_map(|message| serde_json::from_str::<Value>(message).ok())
                .find_map(|message| {
                    message["result"]["protocolVersion"]
                        .as_str()
                        .map(str::to_owned)
                });
        }
        messages
    }

    /// Posts with the connection known, and once more with the settings read again
    /// when that fails.
    fn post(&mut self, body: &str, settings: &Settings<'_>) -> Result<Answer, Failure> {
        if self.connection.is_none() {
            self.connection = Some(settings().map_err(Failure::Settings)?);
        }
        match self.send(body) {
            Err(Failure::Unreachable | Failure::Unauthorized) => {
                self.connection = Some(settings().map_err(Failure::Settings)?);
                self.send(body)
            }
            result => result,
        }
    }

    fn send(&self, body: &str) -> Result<Answer, Failure> {
        let connection = self.connection.as_ref().expect("read before sending");
        let mut request = self
            .http
            .post(address(connection.port))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", &format!("Bearer {}", connection.token));
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(protocol) = &self.protocol {
            request = request.header("MCP-Protocol-Version", protocol);
        }
        let mut response = request.send(body).map_err(|_| Failure::Unreachable)?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let content_type = header("content-type").unwrap_or_default();
        let session = header("mcp-session-id");
        let body = response
            .body_mut()
            .with_config()
            .limit(ANSWER_LIMIT)
            .read_to_string()
            .unwrap_or_default();
        match status {
            200..=299 => Ok(Answer {
                content_type,
                body,
                session,
            }),
            401 => Err(Failure::Unauthorized),
            404 if self.session.is_some() => Err(Failure::SessionGone),
            _ => Err(Failure::Status(status, body)),
        }
    }

    /// Ends the session, so the server forgets it at once. Best effort.
    fn close(&self) {
        let (Some(connection), Some(session)) = (&self.connection, &self.session) else {
            return;
        };
        let _ = self
            .http
            .delete(address(connection.port))
            .header("Authorization", &format!("Bearer {}", connection.token))
            .header("Mcp-Session-Id", session)
            .call();
    }
}

fn reason(failure: &Failure) -> String {
    match failure {
        Failure::Unreachable => {
            "VCMP isn't open, or its server for AI agents isn't running. Open VCMP; Settings, AI agents, says whether the server runs and why not.".into()
        }
        // The relay read the token the settings hold just now, so the VCMP that runs
        // keeps its settings elsewhere.
        Failure::Unauthorized => {
            "VCMP refused the token: the VCMP that runs uses another library than this `vcmp mcp` reads (is VCMP_HOME set for one of them?).".into()
        }
        Failure::SessionGone => {
            "VCMP was restarted or its AI agents switch was turned off and on. Reconnect to start again.".into()
        }
        Failure::Status(status, body) => format!("VCMP answered {status}: {}", body.trim()),
        Failure::Settings(reason) => reason.clone(),
    }
}

fn error_answer(id: &Value, reason: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": RELAY_ERROR, "message": reason },
    })
    .to_string()
}

/// The JSON-RPC messages of an answer, each on one line: a JSON body, or the data of
/// each event of an event stream. Events without data (the stream's priming) are
/// skipped.
fn messages(content_type: &str, body: &str) -> Vec<String> {
    let one_line = |text: &str| {
        serde_json::from_str::<Value>(text)
            .map(|value| value.to_string())
            .ok()
    };
    if !content_type.starts_with("text/event-stream") {
        return one_line(body).into_iter().collect();
    }
    body.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|event| {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect();
            let data = data.join("\n");
            (!data.trim().is_empty()).then(|| one_line(&data)).flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_stream_gives_each_message_on_a_line() {
        let body = "data: \nid: 0\nretry: 3000\n\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":1}\n\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n\n";
        assert_eq!(
            messages("text/event-stream", body),
            [
                r#"{"id":1,"jsonrpc":"2.0"}"#,
                r#"{"jsonrpc":"2.0","method":"x"}"#
            ]
        );
    }

    #[test]
    fn a_json_answer_is_one_line() {
        assert_eq!(
            messages("application/json", "{\n \"id\": 2\n}"),
            [r#"{"id":2}"#]
        );
        assert!(messages("application/json", "").is_empty());
    }
}
