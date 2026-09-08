//! OpenRouter chat-completions client.
//!
//! The request is made from Rust rather than the webview for three
//! reasons: the API key never enters the page, tool calls resolve against
//! the in-memory tree with no IPC round trip, and no CSP or capability
//! change is needed to reach the network.
//!
//! SSE parsing is hand-rolled rather than pulled in as a dependency. The
//! format here is one line of `data: {json}` per chunk terminated by a
//! blank line, plus a `data: [DONE]` sentinel — small enough that a
//! tested 40-line parser is a better trade than another crate, and the
//! tests below cover the cases that actually bite: a JSON object split
//! across two network chunks, comment lines, and garbage.

use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://openrouter.ai/api/v1";

/// Sent as `HTTP-Referer`/`X-Title`, which OpenRouter uses to attribute
/// traffic. Being identifiable is the polite thing for a client that
/// makes requests on a user's behalf.
const APP_URL: &str = "https://github.com/ArpitGoriya/SpaceTree";
const APP_TITLE: &str = "SpaceTree";

// ---------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Set on `role: "tool"` messages to say which call this answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::text("system", content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::text("user", content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text("assistant", content)
    }
    fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }
    pub fn tool_result(call_id: &str, name: &str, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(call_id.to_string()),
            name: Some(name.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_tool_type")]
    pub call_type: String,
    pub function: FunctionCall,
}

fn default_tool_type() -> String {
    "function".to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// A JSON object, as a string. Models stream this in fragments, so it
    /// is only parsed once the stream for that call has finished.
    pub arguments: String,
}

#[derive(Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub def_type: &'static str,
    pub function: FunctionDef,
}

#[derive(Serialize)]
pub struct FunctionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: serde_json::Value,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDef]>,
    temperature: f32,
}

// ---------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------

/// One thing that happened while the model was responding.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// The model finished and wants these tools run.
    ToolCalls(Vec<ToolCall>),
    /// The response ended without tool calls.
    Done,
}

/// Accumulates streamed chunks into whole events.
///
/// Tool calls arrive in pieces — the name in one chunk, the arguments
/// across several more, keyed by an `index` — so this reassembles them
/// rather than treating each chunk as a call.
#[derive(Default)]
pub struct StreamAccumulator {
    text: String,
    calls: Vec<PartialCall>,
    finished: bool,
}

#[derive(Default, Clone)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ChunkEnvelope {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

#[derive(Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Deserialize)]
struct DeltaToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Deserialize)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

impl StreamAccumulator {
    /// Feed one `data:` payload. Returns prose to show immediately, if any.
    ///
    /// An `Err` here is a hard API error (bad key, no credit, unknown
    /// model); an unparseable line is *not* an error — providers routinely
    /// emit keep-alive comments and padding that must be skipped rather
    /// than aborting a working stream.
    pub fn push(&mut self, payload: &str) -> Result<Option<String>, String> {
        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(None);
        }
        if payload == "[DONE]" {
            self.finished = true;
            return Ok(None);
        }
        let Ok(chunk) = serde_json::from_str::<ChunkEnvelope>(payload) else {
            return Ok(None);
        };
        if let Some(err) = chunk.error {
            return Err(err.message);
        }

        let mut emitted = None;
        for choice in chunk.choices {
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    self.text.push_str(&text);
                    emitted = Some(match emitted {
                        Some(prev) => format!("{prev}{text}"),
                        None => text,
                    });
                }
            }
            if let Some(calls) = choice.delta.tool_calls {
                for call in calls {
                    if self.calls.len() <= call.index {
                        self.calls.resize(call.index + 1, PartialCall::default());
                    }
                    let slot = &mut self.calls[call.index];
                    if let Some(id) = call.id {
                        slot.id = id;
                    }
                    if let Some(f) = call.function {
                        if let Some(name) = f.name {
                            slot.name.push_str(&name);
                        }
                        if let Some(args) = f.arguments {
                            slot.arguments.push_str(&args);
                        }
                    }
                }
            }
            if choice.finish_reason.is_some() {
                self.finished = true;
            }
        }
        Ok(emitted)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// What the model ultimately asked for.
    pub fn finish(self) -> StreamEvent {
        let calls: Vec<ToolCall> = self
            .calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .enumerate()
            .map(|(i, c)| ToolCall {
                // Some providers omit the id entirely; a stable synthetic
                // one keeps the follow-up `tool` message addressable.
                id: if c.id.is_empty() {
                    format!("call_{i}")
                } else {
                    c.id
                },
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: c.name,
                    // An empty argument string is not valid JSON; the
                    // no-argument case has to become an empty object.
                    arguments: if c.arguments.trim().is_empty() {
                        "{}".to_string()
                    } else {
                        c.arguments
                    },
                },
            })
            .collect();
        if calls.is_empty() {
            StreamEvent::Done
        } else {
            StreamEvent::ToolCalls(calls)
        }
    }
}

/// Splits a byte stream into SSE `data:` payloads.
///
/// Network chunks land on arbitrary boundaries, so a JSON object is
/// routinely cut in half; this holds the remainder until its newline
/// arrives.
#[derive(Default)]
pub struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    pub fn feed(&mut self, bytes: &str) -> Vec<String> {
        self.buffer.push_str(bytes);
        let mut out = Vec::new();
        while let Some(idx) = self.buffer.find('\n') {
            let line = self.buffer[..idx].trim_end_matches('\r').to_string();
            self.buffer.drain(..=idx);
            if let Some(rest) = line.strip_prefix("data:") {
                out.push(rest.trim().to_string());
            }
            // Everything else — blank separators, `:` keep-alive comments,
            // `event:` and `id:` fields — is deliberately ignored.
        }
        out
    }
}

// ---------------------------------------------------------------------
// Model list
// ---------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelDto {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    /// True when both prompt and completion are priced at zero.
    pub is_free: bool,
    /// Whether the model advertises tool calling. Without it the
    /// assistant cannot inspect folders and falls back to digest-only.
    pub supports_tools: bool,
}

#[derive(Deserialize)]
struct ModelsEnvelope {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    pricing: Option<Pricing>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Pricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

fn is_zero(price: &Option<String>) -> bool {
    price
        .as_deref()
        .map(|p| p.trim().parse::<f64>().map(|v| v == 0.0).unwrap_or(false))
        .unwrap_or(false)
}

pub fn parse_models(body: &str) -> Result<Vec<ModelDto>, String> {
    let env: ModelsEnvelope =
        serde_json::from_str(body).map_err(|e| format!("could not read the model list: {e}"))?;
    let mut models: Vec<ModelDto> = env
        .data
        .into_iter()
        .map(|m| {
            let is_free = m
                .pricing
                .as_ref()
                .map(|p| is_zero(&p.prompt) && is_zero(&p.completion))
                .unwrap_or(false);
            let supports_tools = m
                .supported_parameters
                .as_ref()
                .map(|params| params.iter().any(|p| p == "tools"))
                .unwrap_or(false);
            ModelDto {
                name: m.name.unwrap_or_else(|| m.id.clone()),
                id: m.id,
                context_length: m.context_length.unwrap_or(0),
                is_free,
                supports_tools,
            }
        })
        .collect();
    // Free and tool-capable first — that combination is what this app
    // actually wants, and it is otherwise tedious to find in a list of
    // several hundred.
    models.sort_by(|a, b| {
        b.is_free
            .cmp(&a.is_free)
            .then(b.supports_tools.cmp(&a.supports_tools))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(models)
}

pub async fn fetch_models() -> Result<Vec<ModelDto>, String> {
    let client = reqwest::Client::new();
    let body = client
        .get(format!("{API_BASE}/models"))
        .send()
        .await
        .map_err(|e| format!("could not reach OpenRouter: {e}"))?
        .text()
        .await
        .map_err(|e| format!("could not read OpenRouter's reply: {e}"))?;
    parse_models(&body)
}

// ---------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------

/// Start a streaming chat completion. The caller drives the byte stream.
pub async fn stream_chat(
    api_key: &str,
    model: &str,
    messages: &[Message],
    tools: Option<&[ToolDef]>,
) -> Result<reqwest::Response, String> {
    let client = reqwest::Client::new();
    let request = ChatRequest {
        model,
        messages,
        stream: true,
        tools,
        // Low but not zero: this is analysis of concrete numbers, where
        // invention is the failure mode, but a little variation keeps
        // repeated questions from returning word-for-word answers.
        temperature: 0.2,
    };

    let response = client
        .post(format!("{API_BASE}/chat/completions"))
        .bearer_auth(api_key)
        .header("HTTP-Referer", APP_URL)
        .header("X-Title", APP_TITLE)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("could not reach OpenRouter: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(friendly_error(status, &body));
    }
    Ok(response)
}

/// Turn an HTTP failure into something worth reading. The raw body is a
/// JSON blob whose useful part is one nested string.
fn friendly_error(status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(200).collect());

    match status.as_u16() {
        401 => format!("OpenRouter rejected the API key. Check it in Settings. ({detail})"),
        402 => format!("This model needs credit on your OpenRouter account. ({detail})"),
        404 => format!("That model isn't available. Pick another in Settings. ({detail})"),
        429 => format!(
            "Rate limited by OpenRouter — free models throttle hard. Wait a moment or pick another model. ({detail})"
        ),
        _ => format!("OpenRouter returned {status}: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_reassembles_a_payload_split_across_chunks() {
        let mut d = SseDecoder::default();
        assert!(
            d.feed("data: {\"choi").is_empty(),
            "must wait for the newline"
        );
        let out = d.feed("ces\":[]}\n\n");
        assert_eq!(out, vec!["{\"choices\":[]}"]);
    }

    #[test]
    fn sse_skips_comments_blank_lines_and_other_fields() {
        let mut d = SseDecoder::default();
        let out = d.feed(": keep-alive\n\nevent: message\nid: 7\ndata: {\"a\":1}\n\n");
        assert_eq!(out, vec!["{\"a\":1}"], "only data lines are payloads");
    }

    #[test]
    fn sse_handles_crlf_and_several_payloads_in_one_chunk() {
        let mut d = SseDecoder::default();
        let out = d.feed("data: one\r\ndata: two\r\n");
        assert_eq!(out, vec!["one", "two"]);
    }

    #[test]
    fn garbage_lines_do_not_abort_a_working_stream() {
        let mut acc = StreamAccumulator::default();
        assert_eq!(acc.push("not json at all").unwrap(), None);
        let got = acc
            .push(r#"{"choices":[{"delta":{"content":"hello"}}]}"#)
            .unwrap();
        assert_eq!(got.as_deref(), Some("hello"));
        assert_eq!(acc.text(), "hello");
    }

    #[test]
    fn done_sentinel_ends_the_stream() {
        let mut acc = StreamAccumulator::default();
        acc.push(r#"{"choices":[{"delta":{"content":"hi"}}]}"#)
            .unwrap();
        acc.push("[DONE]").unwrap();
        assert_eq!(acc.finish(), StreamEvent::Done);
    }

    #[test]
    fn an_api_error_in_the_stream_surfaces_as_an_error() {
        let mut acc = StreamAccumulator::default();
        let err = acc
            .push(r#"{"error":{"message":"rate limited"}}"#)
            .unwrap_err();
        assert!(err.contains("rate limited"));
    }

    #[test]
    fn tool_calls_are_reassembled_from_fragments() {
        // This is how models actually stream a call: name first, then the
        // arguments a few characters at a time.
        let mut acc = StreamAccumulator::default();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"list_folder"}}]}}]}"#).unwrap();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]}}]}"#).unwrap();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"C:/Users\"}"}}]}}]}"#).unwrap();

        match acc.finish() {
            StreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].function.name, "list_folder");
                assert_eq!(calls[0].function.arguments, r#"{"path":"C:/Users"}"#);
                assert_eq!(calls[0].id, "c1");
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
    }

    #[test]
    fn two_parallel_tool_calls_stay_separate() {
        let mut acc = StreamAccumulator::default();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"second","arguments":"{}"}}]}}]}"#).unwrap();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"first","arguments":"{}"}}]}}]}"#).unwrap();
        match acc.finish() {
            StreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].function.name, "first");
                assert_eq!(calls[1].function.name, "second");
            }
            other => panic!("expected two calls, got {other:?}"),
        }
    }

    #[test]
    fn a_call_with_no_arguments_still_produces_valid_json() {
        let mut acc = StreamAccumulator::default();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"x","function":{"name":"find_reclaimable"}}]}}]}"#).unwrap();
        match acc.finish() {
            StreamEvent::ToolCalls(calls) => {
                assert_eq!(calls[0].function.arguments, "{}");
                serde_json::from_str::<serde_json::Value>(&calls[0].function.arguments)
                    .expect("empty arguments must still parse as JSON");
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn a_call_missing_its_id_gets_a_usable_one() {
        let mut acc = StreamAccumulator::default();
        acc.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"list_folder","arguments":"{}"}}]}}]}"#).unwrap();
        match acc.finish() {
            StreamEvent::ToolCalls(calls) => assert!(!calls[0].id.is_empty()),
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn free_and_tool_capable_models_sort_first() {
        let body = r#"{"data":[
            {"id":"paid/big","name":"Paid Big","pricing":{"prompt":"0.001","completion":"0.002"},"supported_parameters":["tools"]},
            {"id":"free/chat","name":"Free Chat","pricing":{"prompt":"0","completion":"0"},"supported_parameters":[]},
            {"id":"free/tools","name":"Free Tools","pricing":{"prompt":"0","completion":"0"},"supported_parameters":["tools","temperature"]}
        ]}"#;
        let models = parse_models(body).unwrap();
        assert_eq!(models[0].id, "free/tools");
        assert!(models[0].is_free && models[0].supports_tools);
        assert_eq!(models[1].id, "free/chat");
        assert!(!models[1].supports_tools);
        assert_eq!(models[2].id, "paid/big");
        assert!(!models[2].is_free);
    }

    #[test]
    fn a_model_list_with_missing_fields_still_parses() {
        let models = parse_models(r#"{"data":[{"id":"bare/model"}]}"#).unwrap();
        assert_eq!(models[0].name, "bare/model", "name falls back to the id");
        assert!(!models[0].is_free, "unknown pricing must not read as free");
        assert!(!models[0].supports_tools);
    }
}
