//! The assistant: an agent loop over the scan, streamed to the UI.
//!
//! Shape of a turn:
//!
//! 1. Build a system prompt: how to behave, plus a compact scan digest.
//! 2. Ask OpenRouter, streaming.
//! 3. If the model asks for tools, run them against the in-memory tree,
//!    append the results, and ask again — up to [`MAX_TOOL_ROUNDS`].
//! 4. Stream the final prose to the panel.
//!
//! The tree is never serialized into the prompt. On a 1.2M-node scan that
//! would be millions of tokens; instead the model gets an ~800-token
//! digest and calls tools to look at specific folders, the way a person
//! clicks into them.

pub mod openrouter;
pub mod tools;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{Emitter, Manager};

use openrouter::{Message, StreamAccumulator, StreamEvent};
use st_core::digest;

use crate::state::AppState;

/// How many times the model may call tools before it must answer.
///
/// A small free model will otherwise happily list the same folder six
/// times. Eight rounds is far more than any real question needs, and
/// bounds both the wait and the token spend.
const MAX_TOOL_ROUNDS: usize = 8;

/// Roughly how much conversation to carry. Free models are commonly
/// capped near 8k tokens total, so this leaves room for the digest, the
/// tools schema and a full answer.
const HISTORY_TOKEN_BUDGET: usize = 3000;

const SYSTEM_PROMPT: &str = "\
You are the disk-space assistant inside SpaceTree, a tool that has already scanned \
the user's drive. Your job is to tell them what is using space and what is safe to \
remove.

How to work:
- The scan summary below is already in front of you. Use it before calling anything.
- To look deeper, call the tools. They read the scan that is already in memory, so \
they are instant and free — prefer calling one over guessing.
- When asked what can be deleted, call find_reclaimable first.

Rules that matter:
- Never invent a path, a size or a percentage. Every number you give must have come \
from the summary or a tool result. If you don't know, say so and call a tool.
- The safety notes in tool results are authoritative. If a note says something must \
not be deleted, say that plainly — do not soften it or substitute your own opinion. \
Steering someone away from deleting WinSxS or the Installer cache is a genuinely \
useful answer, not a failure to help.
- You cannot delete anything and must not imply you can. Tell the user what to \
remove and let them do it: they right-click any row in the tree and choose \
'Move to Recycle Bin'.
- Lead with the largest wins. Quote real sizes, and give paths exactly as they \
appear in tool output so the app can link them.
- Be brief. A few sentences and a short list beats an essay.";

/// Emitted to the panel as the turn progresses.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvent {
    pub id: String,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEvent {
    pub message: String,
}

/// One prior exchange, as the frontend remembers it.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTurn {
    pub role: String,
    pub content: String,
}

/// Run one user message to completion, streaming events at the webview.
pub async fn run_turn(
    app: tauri::AppHandle,
    question: String,
    history: Vec<HistoryTurn>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let settings = crate::settings::load(&app);
    if !settings.is_configured() {
        return Err(
            "No OpenRouter API key or model set yet — open Settings to add one.".to_string(),
        );
    }

    // Snapshot everything needed from the scan up front, so the mutex is
    // not held across an await. The digest is a few KB of text.
    let (system, use_tools) = {
        let state = app.state::<AppState>();
        let guard = state.scan.lock().unwrap();
        let scan = guard
            .as_ref()
            .ok_or("No scan is loaded — run a scan first.")?;
        let digest = digest::scan_digest(
            &scan.tree,
            scan.root,
            scan.volume.as_ref(),
            true,
            // Without tools this block is all the model will ever see, so
            // it gets a deeper digest to compensate.
            if settings.model_supports_tools {
                20
            } else {
                40
            },
            if settings.model_supports_tools {
                10
            } else {
                20
            },
        );
        let extra = if settings.model_supports_tools {
            String::new()
        } else {
            // Being explicit beats letting the model hallucinate detail
            // it has no way to obtain.
            format!(
                "\n\nNOTE: this model cannot call tools, so the summary below is all you have. \
Answer from it and say plainly when something would need a closer look.\n\n{}",
                digest::reclaimable(&scan.tree, scan.root, true, 15)
            )
        };
        (
            format!("{SYSTEM_PROMPT}\n\n{digest}{extra}"),
            settings.model_supports_tools,
        )
    };

    let mut messages = vec![Message::system(system)];
    for turn in trim_history(history) {
        messages.push(match turn.role.as_str() {
            "assistant" => Message::assistant(turn.content),
            _ => Message::user(turn.content),
        });
    }
    messages.push(Message::user(question));

    let definitions = tools::definitions();
    let tool_defs = if use_tools {
        Some(definitions.as_slice())
    } else {
        None
    };

    for round in 0..=MAX_TOOL_ROUNDS {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        // On the last round, take the tools away so the model is forced
        // to answer rather than looping forever.
        let round_tools = if round == MAX_TOOL_ROUNDS {
            None
        } else {
            tool_defs
        };

        let response = openrouter::stream_chat(
            &settings.openrouter_api_key,
            &settings.model,
            &messages,
            round_tools,
        )
        .await?;

        let mut decoder = openrouter::SseDecoder::default();
        let mut acc = StreamAccumulator::default();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let bytes = chunk.map_err(|e| format!("the connection dropped mid-answer: {e}"))?;
            let text = String::from_utf8_lossy(&bytes);
            for payload in decoder.feed(&text) {
                match acc.push(&payload) {
                    Ok(Some(delta)) => {
                        let _ = app.emit("ai_delta", delta);
                    }
                    Ok(None) => {}
                    Err(message) => return Err(message),
                }
            }
        }

        // Some models return an empty completion rather than an error,
        // which would otherwise leave the panel showing nothing at all.
        let said_nothing = acc.text().trim().is_empty();

        match acc.finish() {
            StreamEvent::Done => {
                if said_nothing {
                    let _ = app.emit(
                        "ai_delta",
                        "That model returned an empty answer. Free models throttle and truncate under load — try again, or pick a different one in Settings."
                            .to_string(),
                    );
                }
                let _ = app.emit("ai_done", ());
                return Ok(());
            }
            StreamEvent::ToolCalls(calls) => {
                // Record what the model asked for, then answer each call.
                messages.push(Message {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(calls.clone()),
                    tool_call_id: None,
                    name: None,
                });

                for call in calls {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let args: serde_json::Value = serde_json::from_str(&call.function.arguments)
                        .unwrap_or(serde_json::json!({}));
                    let label = tools::describe(&call.function.name, &args);

                    let result = {
                        let state = app.state::<AppState>();
                        let guard = state.scan.lock().unwrap();
                        match guard.as_ref() {
                            Some(scan) => tools::dispatch(
                                &scan.tree,
                                scan.root,
                                true,
                                &call.function.name,
                                &args,
                            ),
                            None => "Error: the scan was cleared.".to_string(),
                        }
                    };

                    let _ = app.emit(
                        "ai_tool_call",
                        ToolEvent {
                            id: call.id.clone(),
                            label,
                            detail: result.clone(),
                        },
                    );
                    messages.push(Message::tool_result(&call.id, &call.function.name, result));
                }
            }
        }
    }

    let _ = app.emit("ai_done", ());
    Ok(())
}

/// Keep recent conversation within budget, newest first.
///
/// Tool results are deliberately not carried across turns: they are by
/// far the bulkiest messages and the least reusable, since the model can
/// simply call the tool again if it needs the data. Dropping them first
/// is what keeps a long conversation affordable on a small free model.
fn trim_history(history: Vec<HistoryTurn>) -> Vec<HistoryTurn> {
    let mut kept: Vec<HistoryTurn> = Vec::new();
    let mut budget = HISTORY_TOKEN_BUDGET;
    for turn in history.into_iter().rev() {
        let cost = turn.content.len() / 4 + 8;
        if cost > budget {
            break;
        }
        budget -= cost;
        kept.push(turn);
    }
    kept.reverse();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, len: usize) -> HistoryTurn {
        HistoryTurn {
            role: role.to_string(),
            content: "x".repeat(len),
        }
    }

    #[test]
    fn history_keeps_the_most_recent_turns() {
        let history = vec![turn("user", 40), turn("assistant", 40), turn("user", 40)];
        let kept = trim_history(history);
        assert_eq!(kept.len(), 3, "a short conversation is kept whole");
    }

    #[test]
    fn an_overlong_history_is_trimmed_from_the_oldest_end() {
        let mut history = vec![turn("user", 40_000)]; // way over budget alone
        history.push(turn("assistant", 40));
        history.push(turn("user", 40));
        let kept = trim_history(history);
        assert_eq!(
            kept.len(),
            2,
            "the huge oldest turn must be dropped, the recent ones kept"
        );
        assert_eq!(kept[0].content.len(), 40);
    }

    #[test]
    fn history_never_exceeds_the_budget() {
        let history: Vec<HistoryTurn> = (0..200).map(|_| turn("user", 400)).collect();
        let kept = trim_history(history);
        let cost: usize = kept.iter().map(|t| t.content.len() / 4 + 8).sum();
        assert!(
            cost <= HISTORY_TOKEN_BUDGET,
            "kept {cost} tokens, budget is {HISTORY_TOKEN_BUDGET}"
        );
    }

    #[test]
    fn an_empty_history_is_fine() {
        assert!(trim_history(Vec::new()).is_empty());
    }

    #[test]
    fn the_system_prompt_forbids_inventing_numbers_and_claiming_deletion() {
        // These two instructions are the difference between useful advice
        // and confidently-wrong advice about someone's drive, so they are
        // pinned rather than left to drift in an edit.
        assert!(SYSTEM_PROMPT.contains("Never invent a path"));
        assert!(SYSTEM_PROMPT.contains("cannot delete anything"));
        assert!(SYSTEM_PROMPT.contains("safety notes in tool results are authoritative"));
    }
}
