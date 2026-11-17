//! Anthropic's Messages API (13 §3): `POST /v1/messages` with
//! `stream: true`. Effort is the extended thinking budget; thinking blocks
//! go back with tool results, as the API requires.

use std::collections::{BTreeMap, VecDeque};

use serde_json::{Value, json};

use crate::client::{ModelEvent, ModelRequest, Part, Role, StopReason, parse_args};

fn block(part: &Part) -> Value {
    match part {
        Part::Text { text } => json!({ "type": "text", "text": text }),
        Part::Image { media_type, data } => json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data },
        }),
        Part::ToolCall { id, name, args, .. } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": args })
        }
        Part::ToolResult {
            id,
            content,
            is_error,
            ..
        } => json!({
            "type": "tool_result",
            "tool_use_id": id,
            "content": content,
            "is_error": is_error,
        }),
        Part::Reasoning { data } => data.clone(),
    }
}

pub(crate) fn body(req: &ModelRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            // Tool results come first in their user message (13 §3).
            let mut parts: Vec<&Part> = m.parts.iter().collect();
            if m.role == Role::User {
                parts.sort_by_key(|p| !matches!(p, Part::ToolResult { .. }));
            }
            json!({
                "role": if m.role == Role::User { "user" } else { "assistant" },
                "content": parts.into_iter().map(block).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_output_tokens,
        "system": req.system,
        "messages": messages,
        "stream": true,
    });
    let mut tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters }))
        .collect();
    if req.web_search {
        tools.push(json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 5 }));
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(budget) = req.effort.anthropic_budget() {
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        body["max_tokens"] = json!(req.max_output_tokens.saturating_add(budget));
    }
    body
}

/// A content block being streamed.
#[derive(Debug)]
enum Open {
    Text,
    Tool {
        id: String,
        name: String,
        json: String,
    },
    Thinking(Value),
}

#[derive(Debug, Default)]
pub(crate) struct State {
    blocks: BTreeMap<u64, Open>,
    input: u64,
    output: u64,
    cached: u64,
    stop: Option<StopReason>,
}

impl State {
    pub(crate) fn event(
        &mut self,
        _name: Option<&str>,
        v: &Value,
        out: &mut VecDeque<ModelEvent>,
    ) -> Result<(), String> {
        let index = v["index"].as_u64().unwrap_or(0);
        match v["type"].as_str().unwrap_or_default() {
            "message_start" => {
                let u = &v["message"]["usage"];
                self.input = u["input_tokens"].as_u64().unwrap_or(0);
                self.cached = u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                self.output = u["output_tokens"].as_u64().unwrap_or(0);
            }
            "content_block_start" => {
                let b = &v["content_block"];
                let open = match b["type"].as_str().unwrap_or_default() {
                    "tool_use" => Open::Tool {
                        id: b["id"].as_str().unwrap_or_default().to_owned(),
                        name: b["name"].as_str().unwrap_or_default().to_owned(),
                        json: String::new(),
                    },
                    "thinking" | "redacted_thinking" => Open::Thinking(b.clone()),
                    _ => Open::Text,
                };
                self.blocks.insert(index, open);
            }
            "content_block_delta" => {
                let d = &v["delta"];
                match (
                    d["type"].as_str().unwrap_or_default(),
                    self.blocks.get_mut(&index),
                ) {
                    ("text_delta", _) => out.push_back(ModelEvent::TextDelta {
                        text: d["text"].as_str().unwrap_or_default().to_owned(),
                    }),
                    ("input_json_delta", Some(Open::Tool { json, .. })) => {
                        json.push_str(d["partial_json"].as_str().unwrap_or_default());
                    }
                    ("citations_delta", _) => {
                        let c = &d["citation"];
                        if let Some(url) = c["url"].as_str() {
                            out.push_back(ModelEvent::Source {
                                url: url.to_owned(),
                                title: c["title"].as_str().unwrap_or(url).to_owned(),
                            });
                        }
                    }
                    ("thinking_delta", Some(Open::Thinking(b))) => {
                        let t = b["thinking"].as_str().unwrap_or_default().to_owned()
                            + d["thinking"].as_str().unwrap_or_default();
                        b["thinking"] = json!(t);
                    }
                    ("signature_delta", Some(Open::Thinking(b))) => {
                        let s = b["signature"].as_str().unwrap_or_default().to_owned()
                            + d["signature"].as_str().unwrap_or_default();
                        b["signature"] = json!(s);
                    }
                    _ => {}
                }
            }
            "content_block_stop" => match self.blocks.remove(&index) {
                Some(Open::Tool { id, name, json }) => out.push_back(ModelEvent::ToolCall {
                    id,
                    name,
                    args: parse_args(&json),
                    extra: None,
                }),
                Some(Open::Thinking(data)) => out.push_back(ModelEvent::Reasoning { data }),
                _ => {}
            },
            "message_delta" => {
                if let Some(o) = v["usage"]["output_tokens"].as_u64() {
                    self.output = o;
                }
                self.stop = Some(match v["delta"]["stop_reason"].as_str() {
                    Some("end_turn" | "stop_sequence") => StopReason::EndTurn,
                    Some("tool_use") => StopReason::ToolUse,
                    Some("max_tokens") => StopReason::MaxTokens,
                    Some("refusal") => StopReason::Refusal,
                    _ => StopReason::Other,
                });
            }
            "message_stop" => {
                out.push_back(ModelEvent::Usage {
                    input: self.input,
                    output: self.output,
                    cached: self.cached,
                });
                out.push_back(ModelEvent::Stop {
                    reason: self.stop.unwrap_or(StopReason::EndTurn),
                });
            }
            "error" => {
                return Err(v["error"]["message"]
                    .as_str()
                    .unwrap_or("the provider reported an error")
                    .to_owned());
            }
            _ => {}
        }
        Ok(())
    }
}
