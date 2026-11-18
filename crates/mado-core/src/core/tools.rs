//! The agent tools in the core (spec 13 §8): each tool is an async job
//! that reads pages through `ai-reader`, acts with trusted input, waits
//! for the page to settle and runs commands as its caller's origin.
//! Control clients reach them as `tool.<name>`; tasks (M5-T12) the same.

use std::time::{Duration, Instant};

use data_encoding::BASE64URL_NOPAD;
use mado_engine::tab::{AgentInput, Modifiers};
use mado_engine::{EngineOp, TabCommand, World};
use mado_types::command::{Command, Origin};
use mado_types::{ErrorCode, MadoError, TabId};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::{Input, Internal, Reply};

/// Settling (13 §8): no main-frame load, and no DOM change for 300 ms, at
/// most 10 seconds.
const QUIET_MS: f64 = 300.0;
const SETTLE_MAX: Duration = Duration::from_secs(10);

fn bad(key: &str) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, key.to_owned())
}

fn later(name: &str) -> MadoError {
    MadoError::new(ErrorCode::Unsupported, "error.ai.toolLater").arg("tool", name.to_owned())
}

/// What a tool job needs: the core's inbox and the caller's origin.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub inputs: mpsc::UnboundedSender<Input>,
    pub origin: Origin,
    pub sensitive: Vec<String>,
    pub allowed: Vec<String>,
    /// The profile's HTTP proxy, `host:port`, which `fetch` uses too.
    pub proxy: Option<String>,
}

impl Ctx {
    fn op(&self, op: EngineOp) {
        let _ = self.inputs.send(Input::Internal(Internal::EngineOp(op)));
    }

    async fn command_as(&self, origin: Origin, cmd: Command) -> Result<Value, MadoError> {
        let (reply, rx) = oneshot::channel();
        let _ = self.inputs.send(Input::Command { origin, cmd, reply });
        rx.await.map_err(|_| MadoError::internal())?
    }

    async fn command(&self, cmd: Value) -> Result<Value, MadoError> {
        let cmd: Command = serde_json::from_value(cmd)
            .map_err(|e| bad("error.control.badCommand").arg("reason", e.to_string()))?;
        self.command_as(self.origin, cmd).await
    }

    async fn state(&self) -> Result<Value, MadoError> {
        Ok(self.command_as(Origin::System, Command::StateGet).await?["state"].take())
    }

    async fn eval(&self, tab: TabId, source: String) -> Result<Value, MadoError> {
        let (tx, rx) = oneshot::channel();
        self.op(EngineOp::Apply(
            tab,
            TabCommand::EvaluateInWorld {
                world: World::Mado,
                source,
                reply: Box::new(move |r| {
                    let _ = tx.send(r);
                }),
            },
        ));
        rx.await.map_err(|_| MadoError::internal())?.map_err(|e| {
            MadoError::new(ErrorCode::EngineUnavailable, "error.ai.pageFailed")
                .arg("reason", e.to_string())
        })
    }

    async fn ai(&self, tab: TabId, call: &str, args: Value) -> Result<Value, MadoError> {
        self.eval(tab, mado_engine::page::ai_call(call, &args))
            .await
    }

    async fn input(&self, tab: TabId, input: AgentInput) -> Result<(), MadoError> {
        let (tx, rx) = oneshot::channel();
        self.op(EngineOp::Apply(
            tab,
            TabCommand::Input {
                input,
                reply: Box::new(move |r| {
                    let _ = tx.send(r);
                }),
            },
        ));
        rx.await.map_err(|_| MadoError::internal())?.map_err(|e| {
            MadoError::new(ErrorCode::EngineUnavailable, "error.ai.inputFailed")
                .arg("reason", e.to_string())
        })
    }

    /// The tab's URL, refusing pages the model may not see (13 §9.4).
    async fn page(&self, tab: TabId) -> Result<String, MadoError> {
        let state = self.state().await?;
        let url = state["tabs"][tab.to_string()]["url"]
            .as_str()
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?
            .to_owned();
        if mado_agent::untrusted::refused(&url, &self.sensitive, &self.allowed).is_some() {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.ai.sensitivePage",
            ));
        }
        Ok(url)
    }

    async fn settle(&self, tab: TabId) {
        let end = Instant::now() + SETTLE_MAX;
        tokio::time::sleep(Duration::from_millis(100)).await;
        while Instant::now() < end {
            let loading = self
                .state()
                .await
                .ok()
                .and_then(|s| s["tabs"][tab.to_string()]["progress"].as_f64())
                .is_some_and(|p| p > 0.0 && p < 1.0);
            let quiet = self
                .ai(tab, "quiet", json!([]))
                .await
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(QUIET_MS);
            if !loading && quiet >= QUIET_MS {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn snapshot_lines(
        &self,
        tab: TabId,
        interactive: bool,
        at: Option<&str>,
    ) -> Result<(Value, Vec<String>), MadoError> {
        let url = self.page(tab).await?;
        let v = self
            .ai(
                tab,
                "snapshot",
                json!([{ "interactive": interactive, "ref": at }]),
            )
            .await?;
        if v["sensitiveField"] == true && !mado_agent::untrusted::allowed(&url, &self.allowed) {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.ai.sensitivePage",
            ));
        }
        let lines: Vec<String> = v["lines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str().map(str::to_owned))
            .collect();
        Ok((v, lines))
    }

    /// A snapshot as tools answer it: wrapped as untrusted (13 §9.5).
    async fn snapshot(
        &self,
        tab: TabId,
        interactive: bool,
        at: Option<&str>,
    ) -> Result<Value, MadoError> {
        let (v, lines) = self.snapshot_lines(tab, interactive, at).await?;
        let url = v["url"].as_str().unwrap_or_default();
        let origin = url::Url::parse(url)
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default();
        let joined = lines.join("\n");
        self.note_read(&origin, &joined);
        Ok(json!({
            "tab": tab,
            "title": v["title"],
            "url": url,
            "snapshot": mado_agent::untrusted::wrap(&origin, &joined),
        }))
    }

    /// Acts, settles, and answers what changed.
    async fn act(
        &self,
        tab: TabId,
        action: impl std::future::Future<Output = Result<Value, MadoError>>,
    ) -> Result<Value, MadoError> {
        let before = self
            .snapshot_lines(tab, false, None)
            .await
            .map(|x| x.1)
            .unwrap_or_default();
        let extra = action.await?;
        if extra.get("covered").is_some_and(|c| !c.is_null())
            || extra.get("found") == Some(&json!(false))
        {
            return Ok(extra);
        }
        self.settle(tab).await;
        let after = self
            .snapshot_lines(tab, false, None)
            .await
            .map(|x| x.1)
            .unwrap_or_default();
        let mut out = json!({ "tab": tab, "changes": mado_agent::tools::diff(&before, &after) });
        if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    }

    /// The point of a ref, or why it cannot be clicked.
    async fn point(
        &self,
        tab: TabId,
        args: &Value,
    ) -> Result<Result<(f64, f64), Value>, MadoError> {
        if let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) {
            return Ok(Ok((x, y)));
        }
        let at = args["ref"]
            .as_str()
            .ok_or_else(|| bad("error.ai.needsRef"))?;
        let l = self.ai(tab, "locate", json!([at])).await?;
        if l["found"] != true {
            return Ok(Err(json!({ "found": false, "ref": at })));
        }
        if !l["covered"].is_null() {
            return Ok(Err(json!({ "covered": l["covered"], "ref": at })));
        }
        Ok(Ok((
            l["x"].as_f64().unwrap_or(0.0),
            l["y"].as_f64().unwrap_or(0.0),
        )))
    }

    /// Asks the person when the action needs it (13 §9); `Ok` to go on.
    async fn guard(
        &self,
        tab: Option<TabId>,
        class: mado_agent::safety::Class,
        url: &str,
        detail: String,
        amount: Option<String>,
    ) -> Result<(), MadoError> {
        let (reply, rx) = oneshot::channel();
        let _ = self.inputs.send(Input::Internal(Internal::ToolGuard {
            ask: super::safety::Ask {
                origin: self.origin,
                tab,
                class,
                url: url.to_owned(),
                detail,
                amount,
            },
            reply,
        }));
        rx.await.map_err(|_| MadoError::internal())?
    }

    fn note_read(&self, from: &str, text: &str) {
        let _ = self.inputs.send(Input::Internal(Internal::ToolRead {
            origin: self.origin,
            from: from.to_owned(),
            text: text.to_owned(),
        }));
    }

    async fn crosses(&self, target: &str, text: &str) -> Option<String> {
        let (reply, rx) = oneshot::channel();
        let _ = self.inputs.send(Input::Internal(Internal::ToolCrosses {
            origin: self.origin,
            target: target.to_owned(),
            text: text.to_owned(),
            reply,
        }));
        rx.await.ok().flatten()
    }

    /// What `ai-reader` says about an element, for the rules of 13 §9.2.
    async fn inspect(&self, tab: TabId, at: &str) -> mado_agent::safety::Element {
        self.ai(tab, "inspect", json!([at]))
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Whether this caller may use `tab` for this tool (13 §7.3).
    async fn may(&self, tab: TabId, read_only: bool, navigates: bool) -> Result<(), MadoError> {
        let (reply, rx) = oneshot::channel();
        let _ = self.inputs.send(Input::Internal(Internal::ToolMayUse {
            origin: self.origin,
            tab,
            read_only,
            navigates,
            reply,
        }));
        rx.await.map_err(|_| MadoError::internal())?
    }
}

fn tab_of(args: &Value) -> Result<TabId, MadoError> {
    serde_json::from_value(args["tab"].clone()).map_err(|_| bad("error.ai.needsTab"))
}

/// Runs a tool and writes its audit row (13 §9.7), as tasks and control
/// clients call it.
pub async fn run_audited(ctx: &Ctx, name: &str, args: Value) -> Result<Value, MadoError> {
    // The audit keeps the call without what was typed (13 §9.7).
    let mut logged = args.clone();
    for key in ["text", "code"] {
        if let Some(t) = logged.get(key).and_then(Value::as_str) {
            logged[key] = json!(format!("({} characters)", t.chars().count()));
        }
    }
    let result = run(ctx, name, args).await;
    let summary = match &result {
        Ok(_) => json!({ "tool": name, "args": logged, "ok": true }),
        Err(e) => json!({ "tool": name, "args": logged, "error": e.message_key }),
    };
    let _ = ctx.inputs.send(Input::Internal(Internal::ToolAudit {
        origin: ctx.origin,
        detail: summary,
    }));
    result
}

/// Runs tool `name` with `args`.
pub async fn run(ctx: &Ctx, name: &str, args: Value) -> Result<Value, MadoError> {
    let tool = mado_agent::tools::find(name)
        .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.control.unknownMethod"))?;
    if args.get("tab").is_some_and(|t| !t.is_null()) {
        let tab = tab_of(&args)?;
        match ctx.may(tab, tool.read_only, tool.name == "navigate").await {
            // `askFirst`: the person decides (13 §7.3).
            Err(e) if e.message_key == "error.ai.needsApproval" => {
                let url = ctx.state().await?["tabs"][tab.to_string()]["url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let detail = format!("Use your tab: {}", tool.name);
                ctx.guard(
                    Some(tab),
                    mado_agent::safety::Class::UserTab,
                    &url,
                    detail,
                    None,
                )
                .await?;
            }
            other => other?,
        }
    }
    match tool.name {
        "open" => {
            let url = args["url"]
                .as_str()
                .ok_or_else(|| bad("error.ai.needsUrl"))?
                .to_owned();
            ctx.guard(None, mado_agent::safety::Class::LeaveScope, &url, format!("Open {url}"), None)
                .await?;
            let (reply, rx) = oneshot::channel();
            let _ = ctx.inputs.send(Input::Internal(Internal::AgentOpen {
                origin: ctx.origin,
                url,
                reply,
            }));
            let tab = rx.await.map_err(|_| MadoError::internal())??;
            ctx.settle(tab).await;
            ctx.snapshot(tab, false, None).await
        }
        "navigate" => {
            let tab = tab_of(&args)?;
            let url = args["url"].as_str().ok_or_else(|| bad("error.ai.needsUrl"))?;
            let cmd = match url {
                "back" => json!({ "type": "tab.back", "tabId": tab }),
                "forward" => json!({ "type": "tab.forward", "tabId": tab }),
                "reload" => json!({ "type": "tab.reload", "tabId": tab, "bypassCache": false }),
                u => {
                    ctx.guard(Some(tab), mado_agent::safety::Class::LeaveScope, u, format!("Go to {u}"), None)
                        .await?;
                    let window = ctx.state().await?["tabs"][tab.to_string()]["windowId"].clone();
                    json!({ "type": "nav.open", "windowId": window, "tabId": tab, "input": u, "disposition": "currentTab", "source": "agent", "ctrlEnter": false })
                }
            };
            ctx.command(cmd).await?;
            ctx.settle(tab).await;
            ctx.snapshot(tab, false, None).await
        }
        "snapshot" => {
            let tab = tab_of(&args)?;
            ctx.snapshot(tab, args["interactive"] == true, args["ref"].as_str()).await
        }
        "text" => {
            let tab = tab_of(&args)?;
            let url = ctx.page(tab).await?;
            let v = ctx.eval(tab, mado_engine::page::ai_text()).await?;
            if v["sensitiveField"] == true && !mado_agent::untrusted::allowed(&url, &ctx.allowed) {
                return Err(MadoError::new(ErrorCode::PermissionDenied, "error.ai.sensitivePage"));
            }
            let origin = url::Url::parse(&url).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
            let (body, _) = mado_agent::untrusted::cap(v["text"].as_str().unwrap_or_default(), 30_000);
            ctx.note_read(&origin, body);
            Ok(json!({ "tab": tab, "title": v["title"], "url": url, "text": mado_agent::untrusted::wrap(&origin, body) }))
        }
        "screenshot" => {
            let tab = tab_of(&args)?;
            ctx.page(tab).await?;
            let (tx, rx) = oneshot::channel();
            ctx.op(EngineOp::Apply(
                tab,
                TabCommand::Snapshot {
                    full_page: false,
                    reply: Box::new(move |r| {
                        let _ = tx.send(r);
                    }),
                },
            ));
            let image = rx
                .await
                .map_err(|_| MadoError::internal())?
                .map_err(|e| MadoError::new(ErrorCode::EngineUnavailable, "error.ai.pageFailed").arg("reason", e.to_string()))?;
            use base64::Engine as _;
            let mime = match image.format {
                mado_engine::tab::ImageFormat::Png => "image/png",
                _ => "image/jpeg",
            };
            Ok(json!({ "tab": tab, "image": { "mimeType": mime, "data": base64::engine::general_purpose::STANDARD.encode(&image.bytes) } }))
        }
        "click" => {
            let tab = tab_of(&args)?;
            let url = ctx.page(tab).await?;
            // 13 §9.2: a consequential click asks first, with the total for
            // a payment.
            if let Some(at) = args["ref"].as_str() {
                let el = ctx.inspect(tab, at).await;
                if let Some(class) = mado_agent::safety::classify_press(&el, &ctx.sensitive) {
                    let amount = (class == mado_agent::safety::Class::Pay).then(|| el.total.clone()).flatten();
                    ctx.guard(Some(tab), class, &url, format!("Click \"{}\"", el.name), amount).await?;
                }
            }
            ctx.act(tab, async {
                match ctx.point(tab, &args).await? {
                    Err(why) => Ok(why),
                    Ok((x, y)) => {
                        ctx.input(tab, AgentInput::Click { x, y, modifiers: Modifiers::default() }).await?;
                        Ok(json!({}))
                    }
                }
            })
            .await
        }
        "hover" => {
            let tab = tab_of(&args)?;
            ctx.page(tab).await?;
            ctx.act(tab, async {
                match ctx.point(tab, &args).await? {
                    Err(why) => Ok(why),
                    Ok((x, y)) => {
                        ctx.input(tab, AgentInput::Hover { x, y }).await?;
                        Ok(json!({}))
                    }
                }
            })
            .await
        }
        "type" => {
            let tab = tab_of(&args)?;
            let url = ctx.page(tab).await?;
            let text = args["text"].as_str().ok_or_else(|| bad("error.ai.needsText"))?.to_owned();
            let target = url::Url::parse(&url).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
            // 13 §9.5: text read on another site needs the person, with its
            // source; 9.1: account settings and, in Ask, any form.
            if let Some(source) = ctx.crosses(&target, &text).await {
                ctx.guard(Some(tab), mado_agent::safety::Class::CrossSiteData, &url, format!("Type text read on {source}"), None)
                    .await?;
            }
            if let Some(at) = args["ref"].as_str() {
                let el = ctx.inspect(tab, at).await;
                if el.account_field {
                    ctx.guard(Some(tab), mado_agent::safety::Class::AccountChange, &url, format!("Change \"{}\"", el.name), None)
                        .await?;
                } else if el.in_form {
                    ctx.guard(Some(tab), mado_agent::safety::Class::TypeInForm, &url, format!("Type into \"{}\"", el.name), None)
                        .await?;
                }
                if args["submit"] == true
                    && let Some(class) = mado_agent::safety::classify_press(&el, &ctx.sensitive)
                {
                    ctx.guard(Some(tab), class, &url, format!("Submit \"{}\"", el.name), el.total.clone()).await?;
                }
            }
            ctx.act(tab, async {
                if let Some(at) = args["ref"].as_str()
                    && ctx.ai(tab, "focus", json!([at])).await? != true
                {
                    return Ok(json!({ "found": false, "ref": at }));
                }
                ctx.input(tab, AgentInput::Text(text)).await?;
                if args["submit"] == true {
                    ctx.input(tab, AgentInput::Key("Enter".into())).await?;
                }
                Ok(json!({}))
            })
            .await
        }
        "choose" => {
            let tab = tab_of(&args)?;
            ctx.page(tab).await?;
            let at = args["ref"].as_str().ok_or_else(|| bad("error.ai.needsRef"))?;
            let value = args["value"].as_str().unwrap_or_default();
            ctx.act(tab, async {
                let chosen = ctx.ai(tab, "choose", json!([at, value])).await?;
                Ok(if chosen.is_null() { json!({ "found": false, "ref": at }) } else { json!({ "chosen": chosen }) })
            })
            .await
        }
        "press" => {
            let tab = tab_of(&args)?;
            ctx.page(tab).await?;
            let keys = args["keys"].as_str().ok_or_else(|| bad("error.ai.needsKeys"))?.to_owned();
            ctx.act(tab, async {
                ctx.input(tab, AgentInput::Key(keys)).await?;
                Ok(json!({}))
            })
            .await
        }
        "scroll" => {
            let tab = tab_of(&args)?;
            ctx.page(tab).await?;
            let (dx, dy) = (args["dx"].as_f64().unwrap_or(0.0), args["dy"].as_f64().unwrap_or(0.0));
            ctx.act(tab, async {
                ctx.ai(tab, "scroll", json!([args["ref"], dx, dy])).await?;
                Ok(json!({}))
            })
            .await
        }
        "drag" => {
            let tab = tab_of(&args)?;
            ctx.page(tab).await?;
            ctx.act(tab, async {
                let from = ctx.point(tab, &json!({ "ref": args["from"] })).await?;
                let to = ctx.point(tab, &json!({ "ref": args["to"] })).await?;
                match (from, to) {
                    (Ok(from), Ok(to)) => {
                        ctx.input(tab, AgentInput::Drag { from, to }).await?;
                        Ok(json!({}))
                    }
                    (Err(why), _) | (_, Err(why)) => Ok(why),
                }
            })
            .await
        }
        "wait" => {
            let tab = tab_of(&args)?;
            let limit = Duration::from_millis(args["timeoutMs"].as_u64().unwrap_or(10_000).min(120_000));
            let end = Instant::now() + limit;
            match args["for"].as_str() {
                Some(wanted) => loop {
                    let (_, lines) = ctx.snapshot_lines(tab, false, None).await?;
                    if lines.iter().any(|l| l.contains(wanted) || l.trim_start().starts_with(&format!("{wanted} "))) {
                        break;
                    }
                    if Instant::now() > end {
                        return Err(MadoError::new(ErrorCode::NotFound, "error.ai.waitTimedOut"));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                },
                None => ctx.settle(tab).await,
            }
            ctx.snapshot(tab, false, None).await
        }
        "sign_in" | "autofill" => {
            let tab = tab_of(&args)?;
            if tool.name == "sign_in" {
                let url = ctx.page(tab).await?;
                ctx.guard(Some(tab), mado_agent::safety::Class::SignIn, &url, "Sign in with your saved password".into(), None)
                    .await?;
            }
            let what = match (tool.name, args["what"].as_str()) {
                ("sign_in", _) => super::fill::AgentFill::SignIn,
                (_, Some("card")) => super::fill::AgentFill::Card,
                (_, Some("address")) => super::fill::AgentFill::Address,
                _ => super::fill::AgentFill::Form,
            };
            let (reply, rx) = oneshot::channel();
            let _ = ctx.inputs.send(Input::Internal(Internal::AgentFill { tab, what, reply }));
            let r = rx.await.map_err(|_| MadoError::internal())??;
            Ok(if tool.name == "sign_in" { json!({ "filled": r["filled"], "username": r["username"] }) } else { json!({ "fields": r["fields"], "left": r["left"] }) })
        }
        "history" => ctx
            .command(json!({ "type": "history.query", "text": args["query"], "host": args["host"], "from": null, "to": null, "limit": 50 }))
            .await,
        "bookmarks" => ctx.command(json!({ "type": "bookmark.search", "query": args["query"].as_str().unwrap_or_default() })).await,
        "bookmark_add" => {
            let url = args["url"].as_str().ok_or_else(|| bad("error.ai.needsUrl"))?;
            ctx.command(json!({ "type": "bookmark.add", "parentId": null, "title": args["title"].as_str().unwrap_or(url), "url": url, "index": null }))
                .await
        }
        "native_context" => {
            let s = ctx.state().await?;
            let tabs: Vec<Value> = s["tabs"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(id, t)| json!({ "tab": id, "window": t["windowId"], "title": t["title"], "url": t["url"], "group": t["groupId"], "pinned": t["pinned"] }))
                .collect();
            Ok(json!({ "windows": s["windows"].as_object().map(|w| w.keys().cloned().collect::<Vec<_>>()), "tabs": tabs, "groups": s["groups"], "features": s["features"] }))
        }
        "native_actions" => Ok(json!(mado_agent::catalog::all()
            .iter()
            .filter(|e| !e.may_need_full())
            .map(|e| json!({ "action": e.name, "params": e.params.iter().map(|(k, t)| json!({ "name": k, "type": t })).collect::<Vec<_>>() }))
            .collect::<Vec<_>>())),
        "native_apply" => {
            let action = args["action"].as_str().ok_or_else(|| bad("error.ai.needsAction"))?;
            ctx.guard(None, mado_agent::safety::Class::Settings, "mado://settings", format!("Change Mado: {action}"), None)
                .await?;
            let mut cmd = args["args"].as_object().cloned().unwrap_or_default();
            cmd.insert("type".into(), json!(action));
            // The undo (13 §8): a setting's previous value, or reopening a
            // closed tab.
            let undo = match action {
                "settings.set" => {
                    let key = cmd.get("key").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let old = ctx.command(json!({ "type": "settings.get", "keys": [key] })).await.ok().map(|v| v[&key].clone());
                    old.map(|o| json!({ "type": "settings.set", "key": key, "value": o, "token": null }))
                }
                "tab.close" => Some(json!({ "type": "session.reopenClosed" })),
                _ => None,
            };
            let result = ctx.command(Value::Object(cmd)).await?;
            let token = undo.map(|u| BASE64URL_NOPAD.encode(u.to_string().as_bytes()));
            Ok(json!({ "result": result, "undoToken": token }))
        }
        "native_undo" => {
            let token = args["token"].as_str().ok_or_else(|| bad("error.ai.badUndo"))?;
            let cmd: Value = BASE64URL_NOPAD
                .decode(token.as_bytes())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .ok_or_else(|| bad("error.ai.badUndo"))?;
            ctx.command(cmd).await
        }
        "close" => {
            let tab = tab_of(&args)?;
            ctx.command(json!({ "type": "tab.close", "tabIds": [tab] })).await
        }
        "fetch" => fetch(ctx, &args).await,
        "run" => {
            let code = args["code"].as_str().ok_or_else(|| bad("error.ai.needsCode"))?.to_owned();
            let ctx = ctx.clone();
            tokio::task::spawn_blocking(move || script(&ctx, &code))
                .await
                .map_err(|_| MadoError::internal())?
        }
        "upload" => {
            let tab = tab_of(&args)?;
            let url = ctx.page(tab).await?;
            let names: Vec<&str> = args["files"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            ctx.guard(Some(tab), mado_agent::safety::Class::Upload, &url, format!("Upload {}", names.join(", ")), None)
                .await?;
            let files: Vec<std::path::PathBuf> = args["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f.as_str().map(std::path::PathBuf::from))
                .collect();
            if files.is_empty() || files.iter().any(|f| !f.is_absolute() || !f.is_file()) {
                return Err(bad("error.ai.badFiles"));
            }
            let _ = ctx.inputs.send(Input::Internal(Internal::PendingUpload { tab, files }));
            ctx.act(tab, async {
                match ctx.point(tab, &args).await? {
                    Err(why) => Ok(why),
                    Ok((x, y)) => {
                        ctx.input(tab, AgentInput::Click { x, y, modifiers: Modifiers::default() }).await?;
                        Ok(json!({}))
                    }
                }
            })
            .await
        }
        "download" => {
            let tab = tab_of(&args)?;
            let page = ctx.page(tab).await?;
            let what = args["url"].as_str().unwrap_or("a link").to_owned();
            ctx.guard(Some(tab), mado_agent::safety::Class::Download, &page, format!("Download {what}"), None)
                .await?;
            if let Some(url) = args["url"].as_str() {
                let src = format!(
                    "const a = document.createElement('a'); a.href = {}; a.download = ''; document.body.append(a); a.click(); a.remove(); return true;",
                    Value::String(url.to_owned())
                );
                ctx.eval(tab, src).await?;
                return Ok(json!({ "tab": tab, "started": true }));
            }
            ctx.act(tab, async {
                match ctx.point(tab, &args).await? {
                    Err(why) => Ok(why),
                    Ok((x, y)) => {
                        let option = Modifiers { option: true, ..Modifiers::default() };
                        ctx.input(tab, AgentInput::Click { x, y, modifiers: option }).await?;
                        Ok(json!({ "started": true }))
                    }
                }
            })
            .await
        }
        other => Err(later(other)),
    }
}

/// The largest `fetch` answer (13 §8).
const FETCH_MAX: usize = 2 * 1024 * 1024;

/// `fetch`: a GET with the profile's cookies for that site, text only.
async fn fetch(ctx: &Ctx, args: &Value) -> Result<Value, MadoError> {
    let url = args["url"]
        .as_str()
        .ok_or_else(|| bad("error.ai.needsUrl"))?
        .to_owned();
    if mado_agent::untrusted::refused(&url, &ctx.sensitive, &ctx.allowed).is_some() {
        return Err(MadoError::new(
            ErrorCode::PermissionDenied,
            "error.ai.sensitivePage",
        ));
    }
    let (tx, rx) = oneshot::channel();
    ctx.op(EngineOp::Cookies(
        url.clone(),
        Box::new(move |c| {
            let _ = tx.send(c);
        }),
    ));
    let cookies = rx.await.unwrap_or_default();
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("Mado/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30));
    if let Some(p) = &ctx.proxy
        && let Ok(proxy) = reqwest::Proxy::all(format!("http://{p}"))
    {
        builder = builder.proxy(proxy);
    }
    let client = builder.build().map_err(|_| MadoError::internal())?;
    let mut req = client.get(&url);
    if !cookies.is_empty() {
        let header: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        req = req.header("cookie", header.join("; "));
    }
    let failed = |e: String| {
        MadoError::new(ErrorCode::NetworkFailed, "error.ai.fetchFailed").arg("reason", e)
    };
    let mut resp = req.send().await.map_err(|e| failed(e.to_string()))?;
    let status = resp.status().as_u16();
    let kind = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .to_ascii_lowercase();
    let texty = kind.starts_with("text/")
        || ["json", "xml", "javascript", "csv"]
            .iter()
            .any(|k| kind.contains(k));
    let unknown = kind.starts_with("application/octet-stream");
    if !texty && !unknown {
        return Err(MadoError::new(ErrorCode::Unsupported, "error.ai.notText").arg("type", kind));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| failed(e.to_string()))? {
        body.extend_from_slice(&chunk);
        if body.len() > FETCH_MAX {
            body.truncate(FETCH_MAX);
            break;
        }
    }
    let body = String::from_utf8_lossy(&body).into_owned();
    let origin = url::Url::parse(&url)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    ctx.note_read(&origin, &body);
    Ok(json!({
        "url": url,
        "status": status,
        "contentType": kind,
        "body": mado_agent::untrusted::wrap(&origin, &body),
    }))
}

/// At most this many tool calls per `run` (13 §8).
const RUN_STEPS: u32 = 200;
/// A `run` script stops after this long.
const RUN_TIME: Duration = Duration::from_secs(600);

/// `run` (13 §8): JavaScript in QuickJS, inside Mado and never in a page,
/// whose functions are the tools. Blocks; runs on a blocking thread.
fn script(ctx: &Ctx, code: &str) -> Result<Value, MadoError> {
    use rquickjs::{Context, Function, Promise, Runtime};
    let failed =
        |e: String| MadoError::new(ErrorCode::InvalidInput, "error.ai.runFailed").arg("reason", e);
    let rt = Runtime::new().map_err(|e| failed(e.to_string()))?;
    rt.set_memory_limit(64 * 1024 * 1024);
    let deadline = Instant::now() + RUN_TIME;
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let context = Context::full(&rt).map_err(|e| failed(e.to_string()))?;
    let local = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| MadoError::internal())?;
    let steps = std::cell::Cell::new(0u32);
    let names: Vec<&str> = mado_agent::tools::all()
        .iter()
        .map(|t| t.name)
        .filter(|n| *n != "run")
        .collect();
    context.with(|qc| {
        let call_ctx = ctx.clone();
        let call = Function::new(qc.clone(), move |name: String, args: String| -> String {
            steps.set(steps.get() + 1);
            if steps.get() > RUN_STEPS {
                return json!({ "error": "error.ai.tooManySteps" }).to_string();
            }
            let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
            match local.block_on(run(&call_ctx, &name, args)) {
                Ok(v) => json!({ "ok": v }).to_string(),
                Err(e) => json!({ "error": e.message_key, "args": e.args }).to_string(),
            }
        })
        .map_err(|e| failed(e.to_string()))?;
        qc.globals().set("__mado", call).map_err(|e| failed(e.to_string()))?;
        let prelude: String = names
            .iter()
            .map(|n| {
                format!(
                    "globalThis[{n:?}] = async (a) => {{ const r = JSON.parse(__mado({n:?}, JSON.stringify(a ?? {{}}))); if (r.error) throw new Error(r.error + ' ' + JSON.stringify(r.args ?? {{}})); return r.ok; }};"
                )
            })
            .collect();
        qc.eval::<(), _>(prelude).map_err(|e| failed(e.to_string()))?;
        let wrapped = format!("(async () => {{\n{code}\n}})().then((v) => JSON.stringify(v === undefined ? null : v))");
        let promise: Promise = qc.eval(wrapped).map_err(|e| {
            let msg = qc.catch().as_exception().map_or_else(|| e.to_string(), |x| x.message().unwrap_or_default());
            failed(msg)
        })?;
        let out: String = promise.finish().map_err(|e| {
            let msg = qc.catch().as_exception().map_or_else(|| e.to_string(), |x| x.message().unwrap_or_default());
            failed(msg)
        })?;
        Ok(serde_json::from_str(&out).unwrap_or(Value::Null))
    })
}

impl super::Core {
    /// What a tool job needs from the core, for `origin`.
    pub(super) fn tool_ctx(&self, origin: Origin) -> Ctx {
        Ctx {
            inputs: self.inputs.clone(),
            origin,
            sensitive: mado_lists::sensitive_hosts().banking.clone(),
            allowed: self
                .setting("ai.allowedSensitiveSites")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            proxy: (self.setting_str("proxy.mode") == "manual")
                .then(|| self.setting_str("proxy.http"))
                .filter(|p| !p.is_empty()),
        }
    }

    /// A tool call from a control client or a task.
    pub(super) fn tool_call(&mut self, origin: Origin, name: String, args: Value, reply: Reply) {
        if self.setting("features.ai") != true
            && !matches!(origin, Origin::Control { .. } | Origin::FullControl { .. })
        {
            let _ = reply.send(Err(MadoError::new(ErrorCode::Unsupported, "error.ai.off")));
            return;
        }
        let ctx = self.tool_ctx(origin);
        tokio::task::spawn_local(async move {
            let result = run_audited(&ctx, &name, args).await;
            let _ = reply.send(result);
        });
    }
}
