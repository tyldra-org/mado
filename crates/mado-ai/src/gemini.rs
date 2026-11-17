//! Google's Gemini API (13 §3): `streamGenerateContent?alt=sse`. Tools
//! are function declarations; function responses follow the calls in
//! order, and each call's `thoughtSignature` goes back with it.

use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use crate::client::{ModelEvent, ModelRequest, Part, Role, StopReason};

/// Gemini takes an OpenAPI subset of JSON Schema: these keys are dropped.
fn schema(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "$schema" | "additionalProperties" | "$defs" | "definitions" | "$id"
                    )
                })
                .map(|(k, v)| (k.clone(), schema(v)))
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(schema).collect()),
        other => other.clone(),
    }
}

fn part(p: &Part) -> Option<Value> {
    Some(match p {
        Part::Text { text } => json!({ "text": text }),
        Part::Image { media_type, data } => {
            json!({ "inlineData": { "mimeType": media_type, "data": data } })
        }
        Part::ToolCall {
            id,
            name,
            args,
            extra,
        } => {
            let mut v = json!({ "functionCall": { "name": name, "args": args, "id": id } });
            if let Some(sig) = extra.as_ref().and_then(|e| e.get("thoughtSignature")) {
                v["thoughtSignature"] = sig.clone();
            }
            v
        }
        Part::ToolResult {
            id,
            name,
            content,
            is_error,
        } => {
            let response = if *is_error {
                json!({ "error": content })
            } else {
                json!({ "result": content })
            };
            json!({ "functionResponse": { "name": name, "id": id, "response": response } })
        }
        Part::Reasoning { .. } => return None,
    })
}

pub(crate) fn body(req: &ModelRequest) -> Value {
    let contents: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            json!({
                "role": if m.role == Role::User { "user" } else { "model" },
                "parts": m.parts.iter().filter_map(part).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut body = json!({
        "systemInstruction": { "parts": [{ "text": req.system }] },
        "contents": contents,
        "generationConfig": { "maxOutputTokens": req.max_output_tokens },
    });
    if !req.tools.is_empty() {
        let decls: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "parameters": schema(&t.parameters) }))
            .collect();
        body["tools"] = json!([{ "functionDeclarations": decls }]);
    }
    if let Some(budget) = req.effort.gemini_budget() {
        body["generationConfig"]["thinkingConfig"] = json!({ "thinkingBudget": budget });
    }
    body
}

#[derive(Debug, Default)]
pub(crate) struct State {
    calls: u32,
    finish: Option<String>,
    usage: Option<(u64, u64, u64)>,
    ended: bool,
}

impl State {
    pub(crate) fn event(
        &mut self,
        v: &Value,
        out: &mut VecDeque<ModelEvent>,
    ) -> Result<(), String> {
        if let Some(e) = v.get("error") {
            return Err(e["message"]
                .as_str()
                .unwrap_or("the provider reported an error")
                .to_owned());
        }
        let c = &v["candidates"][0];
        for p in c["content"]["parts"].as_array().into_iter().flatten() {
            if let Some(f) = p.get("functionCall") {
                self.calls += 1;
                let id = f["id"]
                    .as_str()
                    .map_or_else(|| format!("call_{}", self.calls), str::to_owned);
                out.push_back(ModelEvent::ToolCall {
                    id,
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    args: f.get("args").cloned().unwrap_or_else(|| json!({})),
                    extra: p
                        .get("thoughtSignature")
                        .map(|s| json!({ "thoughtSignature": s })),
                });
            } else if p["thought"] != true
                && let Some(t) = p["text"].as_str()
            {
                out.push_back(ModelEvent::TextDelta { text: t.to_owned() });
            }
        }
        if let Some(f) = c["finishReason"].as_str() {
            self.finish = Some(f.to_owned());
        }
        let u = &v["usageMetadata"];
        if u.is_object() {
            self.usage = Some((
                u["promptTokenCount"].as_u64().unwrap_or(0),
                u["candidatesTokenCount"].as_u64().unwrap_or(0)
                    + u["thoughtsTokenCount"].as_u64().unwrap_or(0),
                u["cachedContentTokenCount"].as_u64().unwrap_or(0),
            ));
        }
        Ok(())
    }

    /// Gemini's stream has no end event: the last chunk carries the finish
    /// reason and usage.
    pub(crate) fn finish(&mut self, out: &mut VecDeque<ModelEvent>) {
        if self.ended || self.finish.is_none() {
            return;
        }
        self.ended = true;
        let (input, output, cached) = self.usage.unwrap_or_default();
        out.push_back(ModelEvent::Usage {
            input,
            output,
            cached,
        });
        let reason = match self.finish.as_deref() {
            Some("STOP") if self.calls > 0 => StopReason::ToolUse,
            Some("STOP") => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some("SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" | "RECITATION") => {
                StopReason::Refusal
            }
            _ => StopReason::Other,
        };
        out.push_back(ModelEvent::Stop { reason });
    }
}
