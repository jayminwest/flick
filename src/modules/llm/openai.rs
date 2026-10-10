//! The `OpenAI` wire, pure: the chat request body, one server-sent-events line of a streamed
//! reply (`chat.completion.chunk`, with content and reasoning deltas, the finish reason, the
//! usage chunk and `[DONE]`), error bodies, and the `/v1/models` list. mlx-serve and ollama
//! both speak it. Nothing here logs or keeps text: it only turns bytes into values.

use serde::Serialize;
use serde_json::Value;

use super::transport::wipe_string;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// One message of a chat, borrowed: building a body copies the text once, into the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Turn<'a> {
    pub role: Role,
    pub content: &'a str,
}

/// A chat request.
#[derive(Clone, Copy, Debug)]
pub struct Chat<'a> {
    pub model: &'a str,
    /// Sent first as a system message unless empty.
    pub system: &'a str,
    pub turns: &'a [Turn<'a>],
    /// 0: not sent (the server's default).
    pub max_tokens: u32,
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    messages: Vec<Turn<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

/// The JSON body of a streamed chat request. The buffer is sized up front so it is not
/// reallocated (and copied) while it grows; the transport wipes it once curl has it.
pub fn chat_body(chat: &Chat) -> Vec<u8> {
    let system = (!chat.system.is_empty()).then_some(Turn { role: Role::System, content: chat.system });
    let messages: Vec<Turn> = system.into_iter().chain(chat.turns.iter().copied()).collect();
    let text: usize = messages.iter().map(|t| t.content.len()).sum();
    let body = Body {
        model: chat.model,
        messages,
        stream: true,
        stream_options: StreamOptions { include_usage: true },
        max_tokens: (chat.max_tokens > 0).then_some(chat.max_tokens),
    };
    // Escapes can double a character; a short text never outgrows this.
    let mut out = Vec::with_capacity(text * 2 + 256);
    // Serializing plain strings and numbers into a Vec cannot fail.
    let _ = serde_json::to_writer(&mut out, &body);
    out
}

/// Token counts of one reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
}

/// What one line of a streamed reply carries, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Answer text.
    Text(String),
    /// Thinking text of a reasoning model (`reasoning_content` or `reasoning`).
    Reasoning(String),
    /// Why the reply ended: `stop`, `length`, ...
    Finish(String),
    Usage(Usage),
    /// `data: [DONE]`: the stream is complete.
    Done,
    /// The server reported an error inside the stream.
    Error(String),
}

impl Piece {
    /// Zero the text this piece holds (best effort, `transport::wipe_string`).
    pub fn wipe(&mut self) {
        match self {
            Piece::Text(s) | Piece::Reasoning(s) | Piece::Finish(s) | Piece::Error(s) => wipe_string(s),
            Piece::Usage(_) | Piece::Done => {}
        }
    }
}

/// Wipe every piece, then empty the list.
pub fn wipe_pieces(pieces: &mut Vec<Piece>) {
    pieces.iter_mut().for_each(Piece::wipe);
    pieces.clear();
}

/// One line of a reply.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// Blank, an SSE comment (`: ping`) or a field the stream does not use (`event:`, `id:`).
    Skip,
    /// A `data:` line and what it carried.
    Data(Vec<Piece>),
    /// Not SSE at all: part of a plain (error) body.
    Other,
}

/// Read one line of a streamed reply (without its line end).
pub fn sse_line(line: &str) -> Line {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.starts_with(':') {
        return Line::Skip;
    }
    let Some(data) = line.strip_prefix("data:") else {
        let field = ["event:", "id:", "retry:"].iter().any(|f| line.starts_with(f));
        return if field { Line::Skip } else { Line::Other };
    };
    let data = data.strip_prefix(' ').unwrap_or(data);
    if data.trim() == "[DONE]" {
        return Line::Data(vec![Piece::Done]);
    }
    let Ok(chunk) = serde_json::from_str::<Value>(data) else {
        return Line::Data(vec![Piece::Error("unreadable stream chunk".into())]);
    };
    Line::Data(pieces(&chunk))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn pieces(chunk: &Value) -> Vec<Piece> {
    if let Some(message) = error_message(chunk) {
        return vec![Piece::Error(message)];
    }
    let mut out = vec![];
    let choice = chunk.get("choices").and_then(|c| c.get(0));
    if let Some(delta) = choice.and_then(|c| c.get("delta")) {
        if let Some(r) = text(delta, "reasoning_content").or_else(|| text(delta, "reasoning")) {
            out.push(Piece::Reasoning(r));
        }
        if let Some(t) = text(delta, "content") {
            out.push(Piece::Text(t));
        }
    }
    if let Some(reason) = choice.and_then(|c| text(c, "finish_reason")) {
        out.push(Piece::Finish(reason));
    }
    if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        out.push(Piece::Usage(Usage { prompt: n("prompt_tokens"), completion: n("completion_tokens") }));
    }
    out
}

/// The message of an error object: `{"error":{"message":..}}`, `{"error":".."}`,
/// `{"detail":".."}` (`FastAPI`) or `{"message":".."}` beside `"error"`.
fn error_message(v: &Value) -> Option<String> {
    let error = v.get("error").filter(|e| !e.is_null());
    let message = match error {
        Some(Value::String(s)) => Some(s.clone()),
        Some(e) => text(e, "message").or_else(|| text(v, "message")).or_else(|| Some(e.to_string())),
        None => text(v, "detail"),
    };
    message.map(|m| cap(m.trim(), 200))
}

/// The error a plain body (not a stream) reports, if it is a JSON error object.
pub fn error_body(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body.trim()).ok().and_then(|v| error_message(&v))
}

/// At most `max` characters of `s`, with `...` when cut.
pub fn cap(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

/// One model a server lists. mlx-serve adds `state`, `context_length` and `capabilities`;
/// ollama lists only ids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Model {
    pub id: String,
    pub state: Option<String>,
    pub context_length: Option<u64>,
    pub capabilities: Vec<String>,
}

/// Read a `/v1/models` body (`{"object":"list","data":[{"id":..},..]}`).
pub fn models(body: &str) -> Result<Vec<Model>, String> {
    let v: Value = serde_json::from_str(body.trim()).map_err(|_| format!("not a model list: {}", cap(body.trim(), 80)))?;
    if let Some(message) = error_message(&v) {
        return Err(message);
    }
    let data = v.get("data").and_then(Value::as_array).ok_or("not a model list: no data array")?;
    Ok(data
        .iter()
        .filter_map(|m| {
            let capabilities = m.get("capabilities").and_then(Value::as_array).map_or_else(Vec::new, |c| {
                c.iter().filter_map(Value::as_str).map(str::to_string).collect()
            });
            Some(Model {
                id: text(m, "id")?,
                state: text(m, "state"),
                context_length: m.get("context_length").and_then(Value::as_u64),
                capabilities,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real mlx-serve captures (fixtures/README.md); the ollama list is still synthetic.
    const STREAM: &str = include_str!("fixtures/mlx_serve_stream.txt");
    const REASONING: &str = include_str!("fixtures/mlx_serve_reasoning.txt");
    const MLX_MODELS: &str = include_str!("fixtures/mlx_serve_models.json");
    const MLX_ERROR: &str = include_str!("fixtures/mlx_serve_error.json");
    const OLLAMA_MODELS: &str = include_str!("fixtures/models_ollama.json");

    fn all(stream: &str) -> Vec<Piece> {
        stream
            .lines()
            .flat_map(|l| match sse_line(l) {
                Line::Data(p) => p,
                Line::Skip => vec![],
                Line::Other => panic!("not sse: {l}"),
            })
            .collect()
    }

    #[test]
    fn a_chat_body_streams_with_usage_and_borrows_the_turns() {
        let turns = [Turn { role: Role::User, content: "hi \"there\"" }, Turn { role: Role::Assistant, content: "yo" }];
        let chat = Chat { model: "qwen", system: "be brief", turns: &turns, max_tokens: 0 };
        let body: Value = serde_json::from_slice(&chat_body(&chat)).unwrap();
        let want = serde_json::json!({
            "model": "qwen",
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi \"there\""},
                {"role": "assistant", "content": "yo"}
            ],
            "stream": true,
            "stream_options": {"include_usage": true}
        });
        assert_eq!(body, want);
        let capped = Chat { system: "", max_tokens: 512, ..chat };
        let body: Value = serde_json::from_slice(&chat_body(&capped)).unwrap();
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_body_never_reallocates_while_it_is_built() {
        let long = "x\"".repeat(4_000);
        let turns = [Turn { role: Role::User, content: &long }];
        let body = chat_body(&Chat { model: "m", system: "", turns: &turns, max_tokens: 0 });
        assert!(body.capacity() == long.len() * 2 + 256, "{} {}", body.len(), body.capacity());
    }

    #[test]
    fn a_stream_reads_into_text_finish_usage_and_done() {
        // mlx-serve sends `"usage":null` on every chunk, a `: keepalive` comment during prefill
        // and `timings` beside the final usage.
        let p = all(STREAM);
        let text: String = p.iter().filter_map(|p| if let Piece::Text(t) = p { Some(t.as_str()) } else { None }).collect();
        assert_eq!(text, "2+2 equals 4.");
        assert_eq!(p.iter().filter(|p| matches!(p, Piece::Usage(_))).count(), 1);
        assert_eq!(p[p.len() - 3..], [Piece::Finish("stop".into()), Piece::Usage(Usage { prompt: 25, completion: 7 }), Piece::Done]);
    }

    #[test]
    fn reasoning_deltas_come_before_the_answer() {
        let p = all(REASONING);
        let answer = p.iter().position(|p| matches!(p, Piece::Text(_))).unwrap();
        assert!(answer > 0 && p[..answer].iter().all(|p| matches!(p, Piece::Reasoning(_))));
        let thought: String = p[..answer].iter().filter_map(|p| if let Piece::Reasoning(r) = p { Some(r.as_str()) } else { None }).collect();
        assert_eq!(thought, "The user asks a simple math question and wants a one-word answer.\n");
        assert_eq!(p[answer..], [Piece::Text("Four".into()), Piece::Finish("stop".into()), Piece::Usage(Usage { prompt: 56, completion: 18 }), Piece::Done]);
        let alt = r#"data: {"choices":[{"delta":{"reasoning":"hm","content":null}}]}"#;
        assert_eq!(sse_line(alt), Line::Data(vec![Piece::Reasoning("hm".into())]));
    }

    #[test]
    fn other_lines_are_skipped_or_marked() {
        for skip in ["", "\r", ": keep-alive", "event: message", "id: 3", "retry: 1000"] {
            assert_eq!(sse_line(skip), Line::Skip, "{skip:?}");
        }
        assert_eq!(sse_line("data:[DONE]\r"), Line::Data(vec![Piece::Done]));
        assert_eq!(sse_line(r#"{"error":"x"}"#), Line::Other);
        assert_eq!(sse_line("data: {not json"), Line::Data(vec![Piece::Error("unreadable stream chunk".into())]));
        assert_eq!(sse_line(r#"data: {"choices":[]}"#), Line::Data(vec![]));
        let null_usage = r#"data: {"choices":[{"delta":{"content":"a"},"finish_reason":null}],"usage":null}"#;
        assert_eq!(sse_line(null_usage), Line::Data(vec![Piece::Text("a".into())]));
    }

    #[test]
    fn errors_in_a_stream_and_in_plain_bodies() {
        let nested = r#"data: {"error":{"message":"model not loaded","type":"x"}}"#;
        assert_eq!(sse_line(nested), Line::Data(vec![Piece::Error("model not loaded".into())]));
        assert_eq!(error_body(MLX_ERROR).as_deref(), Some("Invalid JSON in request body"));
        assert_eq!(error_body(r#"{"error":"boom"}"#).as_deref(), Some("boom"));
        assert_eq!(error_body(r#" {"detail":"Not Found"} "#).as_deref(), Some("Not Found"));
        assert_eq!(error_body(r#"{"error":{"code":5},"message":"bad model"}"#).as_deref(), Some("bad model"));
        assert_eq!(error_body(r#"{"error":{"code":5}}"#).as_deref(), Some(r#"{"code":5}"#));
        assert_eq!(error_body(r#"{"error":null,"ok":1}"#), None);
        assert_eq!(error_body("<html>502</html>"), None);
        let long = format!(r#"{{"error":"{}"}}"#, "é".repeat(300));
        assert_eq!(error_body(&long).unwrap().chars().count(), 203);
    }

    #[test]
    fn wiping_pieces_empties_their_text_and_the_list() {
        let mut p = vec![
            Piece::Text("secret".into()),
            Piece::Reasoning("think".into()),
            Piece::Finish("stop".into()),
            Piece::Error("e".into()),
            Piece::Usage(Usage::default()),
            Piece::Done,
        ];
        let mut one = p[0].clone();
        one.wipe();
        assert_eq!(one, Piece::Text(String::new()));
        for piece in &mut p {
            piece.wipe();
        }
        let empty = |p: &Piece| matches!(p, Piece::Text(s) | Piece::Reasoning(s) | Piece::Finish(s) | Piece::Error(s) if s.is_empty());
        assert!(p[..4].iter().all(empty));
        assert_eq!(p[4..], [Piece::Usage(Usage::default()), Piece::Done]);
        wipe_pieces(&mut p);
        assert!(p.is_empty());
    }

    #[test]
    fn cap_cuts_on_characters() {
        assert_eq!(cap("abc", 3), "abc");
        assert_eq!(cap("abcd", 3), "abc...");
        assert_eq!(cap("ééé", 2), "éé...");
    }

    #[test]
    fn model_lists_from_mlx_serve_and_ollama() {
        let mlx = models(MLX_MODELS).unwrap();
        let want = Model {
            id: "kota-local".into(),
            state: Some("ready".into()),
            context_length: Some(262_144),
            capabilities: ["chat", "tool_use", "streaming", "vision", "reasoning", "json_schema"].map(String::from).to_vec(),
        };
        assert_eq!(mlx, [want]);
        let sparse = models(r#"{"data":[{"id":"g","state":"unloaded","context_length":null}]}"#).unwrap();
        assert_eq!((sparse[0].state.as_deref(), sparse[0].context_length), (Some("unloaded"), None));
        let ollama = models(OLLAMA_MODELS).unwrap();
        let ids: Vec<&str> = ollama.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["llama3.2:3b", "gemma3:12b"]);
        assert!(ollama.iter().all(|m| m.state.is_none() && m.capabilities.is_empty()));
    }

    #[test]
    fn bad_model_lists_say_why() {
        assert_eq!(models(r#"{"data":[{"id":""},{"object":"model"}]}"#).unwrap(), vec![]);
        assert_eq!(models(r#"{"error":{"message":"unauthorized"}}"#).unwrap_err(), "unauthorized");
        assert_eq!(models(r#"{"object":"list"}"#).unwrap_err(), "not a model list: no data array");
        assert_eq!(models("Bad Gateway\n").unwrap_err(), "not a model list: Bad Gateway");
    }
}
