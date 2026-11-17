//! OpenAI's Responses API (13 §3): `POST /responses` with `stream: true`
//! and `store: false`. Reasoning items come back encrypted and go back
//! unchanged, so a stateless conversation keeps them.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::client::{ModelEvent, ModelRequest, Part, Role, StopReason, parse_args};

fn items(req: &ModelRequest) -> Vec<Value> {
    let mut items = Vec::new();
    for m in &req.messages {
        let mut content = Vec::new();
        let flush = |content: &mut Vec<Value>, items: &mut Vec<Value>| {
            if !content.is_empty() {
                let role = if m.role == Role::User {
                    "user"
                } else {
                    "assistant"
                };
                items.push(json!({ "role": role, "content": std::mem::take(content) }));
            }
        };
        for p in &m.parts {
            match p {
                Part::Text { text } if m.role == Role::User => {
                    content.push(json!({ "type": "input_text", "text": text }));
                }
                Part::Text { text } => content.push(json!({ "type": "output_text", "text": text })),
                Part::Image { media_type, data } => content.push(json!({
                    "type": "input_image",
                    "image_url": format!("data:{media_type};base64,{data}"),
                })),
                Part::ToolCall { id, name, args, .. } => {
                    flush(&mut content, &mut items);
                    items.push(json!({
                        "type": "function_call",
                        "call_id": id,
                        "name": name,
                        "arguments": args.to_string(),
                    }));
                }
                Part::ToolResult { id, content: c, .. } => {
                    flush(&mut content, &mut items);
                    items.push(
                        json!({ "type": "function_call_output", "call_id": id, "output": c }),
                    );
                }
                Part::Reasoning { data } => {
                    flush(&mut content, &mut items);
                    items.push(data.clone());
                }
            }
        }
        flush(&mut content, &mut items);
    }
    items
}

pub(crate) fn body(req: &ModelRequest) -> Value {
    let mut body = json!({
        "model": req.model,
        "instructions": req.system,
        "input": items(req),
        "stream": true,
        "store": false,
        "max_output_tokens": req.max_output_tokens,
    });
    let mut tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| json!({ "type": "function", "name": t.name, "description": t.description, "parameters": t.parameters }))
        .collect();
    if req.web_search {
        tools.push(json!({ "type": "web_search" }));
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(effort) = req.effort.openai() {
        body["reasoning"] = json!({ "effort": effort });
        body["include"] = json!(["reasoning.encrypted_content"]);
    }
    body
}

#[derive(Debug, Default)]
pub(crate) struct State {
    called: bool,
}

impl State {
    pub(crate) fn event(
        &mut self,
        v: &Value,
        out: &mut VecDeque<ModelEvent>,
    ) -> Result<(), String> {
        match v["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => out.push_back(ModelEvent::TextDelta {
                text: v["delta"].as_str().unwrap_or_default().to_owned(),
            }),
            "response.output_text.annotation.added" => {
                let a = &v["annotation"];
                if a["type"] == "url_citation"
                    && let Some(url) = a["url"].as_str()
                {
                    out.push_back(ModelEvent::Source {
                        url: url.to_owned(),
                        title: a["title"].as_str().unwrap_or(url).to_owned(),
                    });
                }
            }
            "response.output_item.done" => {
                let item = &v["item"];
                match item["type"].as_str().unwrap_or_default() {
                    "function_call" => {
                        self.called = true;
                        out.push_back(ModelEvent::ToolCall {
                            id: item["call_id"].as_str().unwrap_or_default().to_owned(),
                            name: item["name"].as_str().unwrap_or_default().to_owned(),
                            args: parse_args(item["arguments"].as_str().unwrap_or_default()),
                            extra: None,
                        });
                    }
                    "reasoning" => out.push_back(ModelEvent::Reasoning { data: item.clone() }),
                    _ => {}
                }
            }
            kind @ ("response.completed" | "response.incomplete") => {
                let r = &v["response"];
                let u = &r["usage"];
                out.push_back(ModelEvent::Usage {
                    input: u["input_tokens"].as_u64().unwrap_or(0),
                    output: u["output_tokens"].as_u64().unwrap_or(0),
                    cached: u["input_tokens_details"]["cached_tokens"]
                        .as_u64()
                        .unwrap_or(0),
                });
                let reason = if kind == "response.incomplete" {
                    match r["incomplete_details"]["reason"].as_str() {
                        Some("content_filter") => StopReason::Refusal,
                        _ => StopReason::MaxTokens,
                    }
                } else if self.called {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                };
                out.push_back(ModelEvent::Stop { reason });
            }
            "response.failed" => {
                return Err(v["response"]["error"]["message"]
                    .as_str()
                    .unwrap_or("the response failed")
                    .to_owned());
            }
            "error" => {
                return Err(v["message"]
                    .as_str()
                    .unwrap_or("the provider reported an error")
                    .to_owned());
            }
            _ => {}
        }
        Ok(())
    }
}
