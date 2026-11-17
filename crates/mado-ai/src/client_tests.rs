//! Recorded-response tests (M5-T02): a tool-calling conversation round
//! trips through each client against the fixtures in `fixtures/`, and the
//! second request carries the tool result in that provider's format.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::client::{
    Effort, Message, ModelClient, ModelRequest, Part, Role, StopReason, ToolSpec, collect,
};
use crate::connection::{Failure, Provider};
use crate::mock::{Answer, Mock, Request};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/fixtures/{name}.sse",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("a fixture")
}

fn sse(body: String) -> Answer {
    Answer {
        status: 200,
        body,
        headers: Vec::new(),
    }
}

fn request(effort: Effort) -> ModelRequest {
    ModelRequest {
        model: "m-1".into(),
        system: "You are Mado.".into(),
        messages: vec![Message::user("What is the page's title?")],
        tools: vec![ToolSpec {
            name: "page_title".into(),
            description: "The title of a tab.".into(),
            parameters: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "tab": { "type": "string" } },
                "required": ["tab"],
                "additionalProperties": false,
            }),
        }],
        effort,
        max_output_tokens: 1000,
        web_search: false,
    }
}

/// Runs the two turns against `api`'s fixtures; returns both request
/// bodies and the paths they went to.
async fn round_trip(provider: Provider, api: &'static str, effort: Effort) -> Vec<Request> {
    let seen: Arc<Mutex<Vec<Request>>> = Arc::default();
    let log = Arc::clone(&seen);
    let mock = Mock::start(move |r: &Request| {
        log.lock().expect("lock").push(r.clone());
        if r.body.contains("TITLE-RESULT") {
            sse(fixture(&format!("{api}-text")))
        } else {
            sse(fixture(&format!("{api}-tool")))
        }
    })
    .await;
    let base = match provider {
        Provider::Anthropic | Provider::Google => mock.base(),
        _ => format!("{}/v1", mock.base()),
    };
    let client = ModelClient::new(provider, Some(&base), Some("key-1".into())).expect("a client");
    let mut req = request(effort);
    let mut streamed = String::new();
    let mut stream = client.stream(&req).await.expect("started");
    let first = collect(&mut stream, |t| streamed.push_str(t))
        .await
        .expect("the first turn");
    assert_eq!(first.stop, StopReason::ToolUse, "{api}");
    assert_eq!(first.text, "Let me look.", "{api}");
    assert_eq!(streamed, "Let me look.", "{api}");
    let calls = first.tool_calls();
    assert_eq!(calls.len(), 1, "{api}");
    assert_eq!(
        (calls[0].1, calls[0].2),
        ("page_title", &json!({ "tab": "t1" })),
        "{api}"
    );
    assert_eq!(first.usage.0, 120, "{api}");
    let call_id = calls[0].0.to_owned();
    req.messages.push(first.message.clone());
    req.messages.push(Message {
        role: Role::User,
        parts: vec![
            Part::text("Thanks."),
            Part::ToolResult {
                id: call_id,
                name: "page_title".into(),
                content: "TITLE-RESULT Example Domain".into(),
                is_error: false,
            },
        ],
    });
    let mut stream = client.stream(&req).await.expect("started");
    let second = collect(&mut stream, |_| {}).await.expect("the second turn");
    assert_eq!(
        (
            second.text.as_str(),
            second.stop,
            second.usage.0,
            second.usage.1
        ),
        ("The title is Example Domain.", StopReason::EndTurn, 200, 7),
        "{api}"
    );
    let seen = seen.lock().expect("lock").clone();
    assert_eq!(seen.len(), 2, "{api}");
    seen
}

fn body(r: &Request) -> Value {
    serde_json::from_str(&r.body).expect("a JSON body")
}

#[tokio::test]
async fn anthropic_round_trips_a_tool_call() {
    let seen = round_trip(Provider::Anthropic, "anthropic", Effort::Medium).await;
    let r = &seen[1];
    assert_eq!(r.path, "/v1/messages");
    assert_eq!(
        (
            r.headers["x-api-key"].as_str(),
            r.headers["anthropic-version"].as_str()
        ),
        ("key-1", "2023-06-01")
    );
    assert!(!r.headers.contains_key("cookie"));
    let b = body(r);
    assert_eq!(
        (b["stream"].clone(), b["system"].clone()),
        (json!(true), json!("You are Mado."))
    );
    assert_eq!(
        b["thinking"],
        json!({ "type": "enabled", "budget_tokens": 8192 })
    );
    assert_eq!(b["max_tokens"], json!(9192));
    assert_eq!(b["tools"][0]["input_schema"]["required"], json!(["tab"]));
    // The thinking block goes back first, unchanged, then text and the call.
    let assistant = &b["messages"][1]["content"];
    assert_eq!(assistant[0]["type"], "thinking");
    assert_eq!(
        (
            assistant[0]["thinking"].as_str(),
            assistant[0]["signature"].as_str()
        ),
        (Some("Need the title."), Some("sig-abc"))
    );
    assert_eq!(
        assistant[2],
        json!({ "type": "tool_use", "id": "toolu_1", "name": "page_title", "input": { "tab": "t1" } })
    );
    // The tool result comes first in the next user message.
    let user = &b["messages"][2]["content"];
    assert_eq!(user[0]["type"], "tool_result");
    assert_eq!(user[0]["tool_use_id"], "toolu_1");
    assert_eq!(user[1]["text"], "Thanks.");
}

#[tokio::test]
async fn openai_responses_round_trip_a_tool_call() {
    let seen = round_trip(Provider::Openai, "openai", Effort::Xhigh).await;
    let r = &seen[1];
    assert_eq!(r.path, "/v1/responses");
    assert_eq!(r.headers["authorization"], "Bearer key-1");
    let b = body(r);
    assert_eq!(
        (
            b["store"].clone(),
            b["instructions"].clone(),
            b["reasoning"].clone()
        ),
        (
            json!(false),
            json!("You are Mado."),
            json!({ "effort": "high" })
        )
    );
    assert_eq!(b["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(b["tools"][0]["type"], "function");
    let input = b["input"].as_array().expect("items");
    let kinds: Vec<&str> = input
        .iter()
        .map(|i| {
            i["type"]
                .as_str()
                .or(i["role"].as_str())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "user",
            "reasoning",
            "assistant",
            "function_call",
            "user",
            "function_call_output"
        ]
    );
    assert_eq!(input[1]["encrypted_content"], "enc-xyz");
    assert_eq!(input[3]["call_id"], "call_1");
    assert_eq!(input[3]["arguments"], r#"{"tab":"t1"}"#);
    assert_eq!(input[5]["call_id"], "call_1");
}

#[tokio::test]
async fn gemini_round_trips_a_tool_call() {
    let seen = round_trip(Provider::Google, "gemini", Effort::Low).await;
    let r = &seen[1];
    assert_eq!(r.path, "/v1beta/models/m-1:streamGenerateContent?alt=sse");
    assert_eq!(r.headers["x-goog-api-key"], "key-1");
    let b = body(r);
    assert_eq!(
        b["generationConfig"]["thinkingConfig"],
        json!({ "thinkingBudget": 1024 })
    );
    let params = &b["tools"][0]["functionDeclarations"][0]["parameters"];
    assert!(params.get("$schema").is_none() && params.get("additionalProperties").is_none());
    assert_eq!(b["systemInstruction"]["parts"][0]["text"], "You are Mado.");
    let model = &b["contents"][1];
    assert_eq!(model["role"], "model");
    assert_eq!(model["parts"][1]["functionCall"]["name"], "page_title");
    assert_eq!(model["parts"][1]["thoughtSignature"], "sig-g");
    let user = &b["contents"][2]["parts"];
    assert_eq!(user[1]["functionResponse"]["name"], "page_title");
    assert_eq!(
        user[1]["functionResponse"]["response"]["result"],
        "TITLE-RESULT Example Domain"
    );
}

#[tokio::test]
async fn openai_compatible_servers_round_trip_a_tool_call() {
    let seen = round_trip(Provider::Openrouter, "compat", Effort::Low).await;
    let r = &seen[1];
    assert_eq!(r.path, "/v1/chat/completions");
    assert_eq!(r.headers["authorization"], "Bearer key-1");
    let b = body(r);
    assert_eq!(b["reasoning"], json!({ "effort": "low" }));
    assert_eq!(b["stream_options"], json!({ "include_usage": true }));
    let m = b["messages"].as_array().expect("messages");
    let roles: Vec<&str> = m
        .iter()
        .map(|x| x["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "user"]);
    assert_eq!(
        m[2]["tool_calls"][0]["function"]["arguments"],
        r#"{"tab":"t1"}"#
    );
    assert_eq!(m[3]["tool_call_id"], "call_1");
    // Other compatible servers get no effort field.
    let ollama = round_trip(Provider::Ollama, "compat", Effort::High).await;
    assert!(body(&ollama[0]).get("reasoning").is_none());
}

#[tokio::test]
async fn retries_only_before_the_first_byte() {
    let count = Arc::new(Mutex::new(0u32));
    let n = Arc::clone(&count);
    let mock = Mock::start(move |r: &Request| {
        let mut n = n.lock().expect("lock");
        *n += 1;
        match (r.body.contains("busy"), r.body.contains("limited"), *n) {
            (true, _, 1) => Answer::from((503, String::new())),
            (_, true, 1) => Answer {
                status: 429,
                body: "slow down".into(),
                headers: vec![("retry-after".into(), "0".into())],
            },
            (_, true, _) | (true, _, _) => sse(fixture("compat-text")),
            _ => Answer::from((401, r#"{"error":"bad key"}"#.to_owned())),
        }
    })
    .await;
    let mut client = ModelClient::new(
        Provider::OpenaiCompatible,
        Some(&format!("{}/v1", mock.base())),
        None,
    )
    .expect("a client");
    client.backoff = [Duration::from_millis(1), Duration::from_millis(1)];
    let ask = |text: &str| ModelRequest {
        messages: vec![Message::user(text)],
        tools: Vec::new(),
        ..request(Effort::Auto)
    };
    for word in ["busy", "limited"] {
        *count.lock().expect("lock") = 0;
        let mut s = client.stream(&ask(word)).await.expect("retried");
        assert_eq!(
            collect(&mut s, |_| {}).await.expect("answered").text,
            "The title is Example Domain."
        );
        assert_eq!(*count.lock().expect("lock"), 2, "{word}");
    }
    *count.lock().expect("lock") = 0;
    let err = client.stream(&ask("other")).await.expect_err("refused");
    assert_eq!(err.failure(), Failure::Unauthorized);
    assert_eq!(
        *count.lock().expect("lock"),
        1,
        "a refused key is not retried"
    );
}

#[tokio::test]
async fn a_broken_or_failed_stream_is_an_error() {
    let mock = Mock::start(|r: &Request| {
        if r.body.contains("cut") {
            sse("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hal\"}}]}\n\n".into())
        } else {
            sse("event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n".into())
        }
    })
    .await;
    let compat =
        ModelClient::new(Provider::OpenaiCompatible, Some(&mock.base()), None).expect("client");
    let mut s = compat
        .stream(&ModelRequest {
            messages: vec![Message::user("cut")],
            ..request(Effort::Auto)
        })
        .await
        .expect("started");
    assert!(
        collect(&mut s, |_| {}).await.is_err(),
        "an answer without its end"
    );
    let anthropic = ModelClient::new(Provider::Anthropic, Some(&mock.base()), Some("k".into()))
        .expect("client");
    let mut s = anthropic
        .stream(&request(Effort::Auto))
        .await
        .expect("started");
    let err = collect(&mut s, |_| {}).await.expect_err("overloaded");
    assert!(err.to_string().contains("Overloaded"));
}

#[test]
fn clients_need_an_http_provider() {
    assert!(ModelClient::new(Provider::ClaudeCode, None, None).is_err());
    assert!(
        ModelClient::new(Provider::OpenaiCompatible, Some("http://example.com"), None).is_err()
    );
}

#[tokio::test]
async fn web_search_adds_the_providers_tool_and_reads_its_sources() {
    let streams = [
        (
            Provider::Anthropic,
            "/v1/messages",
            concat!(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"citations_delta\",\"citation\":{\"type\":\"web_search_result_location\",\"url\":\"https://a.example/\",\"title\":\"A\",\"cited_text\":\"x\"}}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Found.\"}}\n\n",
                "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ),
        ),
        (
            Provider::Openai,
            "/v1/responses",
            concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Found.\"}\n\n",
                "data: {\"type\":\"response.output_text.annotation.added\",\"annotation\":{\"type\":\"url_citation\",\"url\":\"https://a.example/\",\"title\":\"A\"}}\n\n",
                "data: {\"type\":\"response.output_text.annotation.added\",\"annotation\":{\"type\":\"url_citation\",\"url\":\"https://a.example/\",\"title\":\"A\"}}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}\n\n"
            ),
        ),
        (
            Provider::Google,
            "/v1beta/models/m-1:streamGenerateContent?alt=sse",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Found.\"}]},\"finishReason\":\"STOP\",\"groundingMetadata\":{\"groundingChunks\":[{\"web\":{\"uri\":\"https://a.example/\",\"title\":\"A\"}}]}}]}\n\n",
        ),
    ];
    for (provider, path, stream) in streams {
        let seen: Arc<Mutex<Vec<Request>>> = Arc::default();
        let log = Arc::clone(&seen);
        let body = stream.to_owned();
        let mock = Mock::start(move |r: &Request| {
            log.lock().expect("lock").push(r.clone());
            sse(body.clone())
        })
        .await;
        let base = if provider == Provider::Openai {
            format!("{}/v1", mock.base())
        } else {
            mock.base()
        };
        let client = ModelClient::new(provider, Some(&base), Some("k".into())).expect("client");
        let req = ModelRequest {
            tools: Vec::new(),
            web_search: true,
            ..request(Effort::Auto)
        };
        let mut s = client.stream(&req).await.expect("started");
        let turn = collect(&mut s, |_| {}).await.expect("answered");
        assert_eq!(
            (turn.text.as_str(), turn.sources.clone()),
            (
                "Found.",
                vec![("https://a.example/".to_owned(), "A".to_owned())]
            ),
            "{provider:?}"
        );
        let r = seen.lock().expect("lock")[0].clone();
        assert_eq!(r.path, path);
        let tools = body_of(&r)["tools"].to_string();
        assert!(
            tools.contains("web_search") || tools.contains("google_search"),
            "{provider:?}: {tools}"
        );
    }
}

fn body_of(r: &Request) -> Value {
    serde_json::from_str(&r.body).expect("a JSON body")
}
