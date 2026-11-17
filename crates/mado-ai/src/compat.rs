//! OpenAI-compatible chat completions (13 §3): xAI, Mistral, DeepSeek,
//! Groq, OpenRouter, Cloudflare, Ollama, LM Studio and custom servers.

use std::collections::{BTreeMap, VecDeque};

use serde_json::{Value, json};

use crate::client::{ModelEvent, ModelRequest, Part, Role, StopReason, parse_args};
use crate::connection::Provider;

fn messages(req: &ModelRequest) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": req.system })];
    for m in &req.messages {
        match m.role {
            Role::User => {
                let mut content = Vec::new();
                for p in &m.parts {
                    match p {
                        // Tool results are messages of their own.
                        Part::ToolResult { id, content: c, .. } => out.push(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": c,
                        })),
                        Part::Text { text } => {
                            content.push(json!({ "type": "text", "text": text }))
                        }
                        Part::Image { media_type, data } => content.push(json!({
                            "type": "image_url",
                            "image_url": { "url": format!("data:{media_type};base64,{data}") },
                        })),
                        _ => {}
                    }
                }
                if !content.is_empty() {
                    out.push(json!({ "role": "user", "content": content }));
                }
            }
            Role::Assistant => {
                let text: String = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                let calls: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolCall { id, name, args, .. } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": args.to_string() },
                        })),
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { json!(text) },
                });
                if !calls.is_empty() {
                    msg["tool_calls"] = json!(calls);
                }
                out.push(msg);
            }
        }
    }
    out
}

pub(crate) fn body(req: &ModelRequest, provider: Provider) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": messages(req),
        "stream": true,
        "stream_options": { "include_usage": true },
        "max_tokens": req.max_output_tokens,
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } }))
            .collect();
    }
    // Only OpenRouter takes one effort field for every model.
    if provider == Provider::Openrouter
        && let Some(effort) = req.effort.openai()
    {
        body["reasoning"] = json!({ "effort": effort });
    }
    body
}

#[derive(Debug, Default)]
struct Call {
    id: String,
    name: String,
    args: String,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    calls: BTreeMap<u64, Call>,
    finish: Option<String>,
    usage: Option<(u64, u64, u64)>,
    ended: bool,
}

impl State {
    pub(crate) fn event(
        &mut self,
        v: &Value,
        _out: &mut VecDeque<ModelEvent>,
    ) -> Result<(), String> {
        if let Some(e) = v.get("error") {
            return Err(e["message"]
                .as_str()
                .unwrap_or("the provider reported an error")
                .to_owned());
        }
        let c = &v["choices"][0];
        let d = &c["delta"];
        if let Some(t) = d["content"].as_str()
            && !t.is_empty()
        {
            _out.push_back(ModelEvent::TextDelta { text: t.to_owned() });
        }
        for tc in d["tool_calls"].as_array().into_iter().flatten() {
            let call = self
                .calls
                .entry(tc["index"].as_u64().unwrap_or(0))
                .or_default();
            if let Some(id) = tc["id"].as_str() {
                call.id = id.to_owned();
            }
            if let Some(n) = tc["function"]["name"].as_str() {
                call.name.push_str(n);
            }
            if let Some(a) = tc["function"]["arguments"].as_str() {
                call.args.push_str(a);
            }
        }
        if let Some(f) = c["finish_reason"].as_str() {
            self.finish = Some(f.to_owned());
        }
        let u = &v["usage"];
        if u.is_object() {
            self.usage = Some((
                u["prompt_tokens"].as_u64().unwrap_or(0),
                u["completion_tokens"].as_u64().unwrap_or(0),
                u["prompt_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .unwrap_or(0),
            ));
        }
        Ok(())
    }

    /// `[DONE]` or the end of the stream.
    pub(crate) fn finish(&mut self, out: &mut VecDeque<ModelEvent>) {
        if self.ended || (self.finish.is_none() && self.calls.is_empty() && self.usage.is_none()) {
            return;
        }
        self.ended = true;
        let had_calls = !self.calls.is_empty();
        for (i, call) in std::mem::take(&mut self.calls) {
            out.push_back(ModelEvent::ToolCall {
                id: if call.id.is_empty() {
                    format!("call_{i}")
                } else {
                    call.id
                },
                name: call.name,
                args: parse_args(&call.args),
                extra: None,
            });
        }
        if let Some((input, output, cached)) = self.usage {
            out.push_back(ModelEvent::Usage {
                input,
                output,
                cached,
            });
        }
        let reason = match self.finish.as_deref() {
            Some("tool_calls" | "function_call") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal,
            Some("stop") if had_calls => StopReason::ToolUse,
            Some("stop") | None => StopReason::EndTurn,
            Some(_) => StopReason::Other,
        };
        out.push_back(ModelEvent::Stop { reason });
    }
}
