//! Reader (spec 08 §10): the `reader` probe after each page load, the
//! article from Mozilla Readability sanitised with `ammonia`, shown at
//! `mado://reader?tab=<id>` with the reader settings, and "Always open in
//! Reader" per site in `reader_sites`.

use std::collections::BTreeSet;

use mado_engine::{EngineOp, TabCommand, World};
use mado_types::command::Command;
use mado_types::{ErrorCode, MadoError, TabId};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::Value;

use super::{Core, Internal, lookup_def};

/// What the `reader` script answers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub title: String,
    #[serde(default)]
    pub byline: Option<String>,
    pub content: String,
    #[serde(default)]
    pub length: u64,
    #[serde(default)]
    pub site_name: Option<String>,
}

/// The reader settings the page is drawn with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub font: String,
    pub size: i64,
    pub width: String,
    pub theme: String,
}

/// Hosts set to always open in Reader.
pub fn load(c: &Connection) -> rusqlite::Result<BTreeSet<String>> {
    c.prepare("SELECT host FROM reader_sites WHERE always = 1")?
        .query_map([], |r| r.get(0))?
        .collect()
}

/// Whether `url` is a reader page.
pub fn is_reader_url(url: &str) -> bool {
    url.starts_with("mado://reader?")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The article's HTML with only text, links, images, figures, lists,
/// tables and code left (spec 08 §10).
pub fn sanitise(content: &str) -> String {
    ammonia::Builder::default()
        .tags(
            [
                "a",
                "abbr",
                "b",
                "blockquote",
                "br",
                "caption",
                "code",
                "dd",
                "del",
                "div",
                "dl",
                "dt",
                "em",
                "figcaption",
                "figure",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "hr",
                "i",
                "img",
                "ins",
                "kbd",
                "li",
                "mark",
                "ol",
                "p",
                "pre",
                "q",
                "s",
                "samp",
                "section",
                "small",
                "span",
                "strong",
                "sub",
                "sup",
                "table",
                "tbody",
                "td",
                "tfoot",
                "th",
                "thead",
                "time",
                "tr",
                "u",
                "ul",
            ]
            .into(),
        )
        .generic_attributes(
            BTreeSet::from(["title", "lang", "dir"])
                .into_iter()
                .collect(),
        )
        .add_tag_attributes("a", ["href"])
        .add_tag_attributes("img", ["src", "alt", "width", "height"])
        .add_tag_attributes("td", ["colspan", "rowspan"])
        .add_tag_attributes("th", ["colspan", "rowspan", "scope"])
        .add_tag_attributes("time", ["datetime"])
        .url_schemes(["http", "https", "mailto"].into())
        .link_rel(Some("noopener noreferrer"))
        .clean(content)
        .to_string()
}

/// The reader page: no script may run in it.
pub fn page(article: &Article, source: &str, look: &Look) -> String {
    let (fg, bg) = match look.theme.as_str() {
        "light" => ("#1d1d1f", "#ffffff"),
        "sepia" => ("#5b4636", "#f4ecd8"),
        "dark" => ("#e6e6e6", "#1e1e1e"),
        _ => (
            "light-dark(#1d1d1f, #e6e6e6)",
            "light-dark(#ffffff, #1e1e1e)",
        ),
    };
    let font = if look.font == "sans" {
        "-apple-system, system-ui, sans-serif"
    } else {
        "\"New York\", ui-serif, Georgia, serif"
    };
    let width = match look.width.as_str() {
        "narrow" => "32em",
        "wide" => "48em",
        _ => "40em",
    };
    let byline = article
        .byline
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .map(|b| format!("<p class=\"byline\">{}</p>", escape(b)))
        .unwrap_or_default();
    let site = article.site_name.as_deref().map_or_else(
        || {
            url::Url::parse(source)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default()
        },
        str::to_owned,
    );
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src https: http: data:; style-src 'unsafe-inline'"><meta name="color-scheme" content="light dark"><meta name="referrer" content="no-referrer"><title>{title}</title><style>body{{margin:0;background:{bg};color:{fg};font:{size}px/1.6 {font}}}article{{max-width:{width};margin:0 auto;padding:3em 1.5em}}h1{{font-size:1.8em;line-height:1.2}}.site,.byline{{opacity:.7;font-size:.85em}}img,video,figure{{max-width:100%;height:auto}}pre{{overflow:auto}}a{{color:inherit}}</style></head><body><article><p class="site"><a href="{href}">{site}</a></p><h1>{title}</h1>{byline}{content}</article></body></html>"#,
        title = escape(&article.title),
        href = escape(source),
        site = escape(&site),
        content = sanitise(&article.content),
        size = look.size,
    )
}

impl Core {
    fn reader_on(&self) -> bool {
        self.settings.get::<mado_settings::FeaturesReader>()
    }

    fn look(&self) -> Look {
        let s = |k: &str| {
            self.settings
                .value(lookup_def(k))
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        Look {
            font: s("reader.font"),
            size: self.settings.get::<mado_settings::ReaderSize>(),
            width: s("reader.width"),
            theme: s("reader.theme"),
        }
    }

    /// After a commit: a reader page marks the tab in Reader; any other
    /// document has no article until the probe says so.
    pub(super) fn reader_committed(&mut self, tab: TabId) {
        if let Some(t) = self.model.tabs.get_mut(&tab) {
            t.reader_active = is_reader_url(&t.url);
            t.reader_available = t.reader_active;
            if !t.reader_active {
                self.reader_sources.remove(&tab);
            }
        }
    }

    /// Asks the page whether it looks like an article (after it loaded).
    pub(super) fn reader_probe(&mut self, tab: TabId) {
        let Some(url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) else {
            return;
        };
        if !self.reader_on()
            || self.error_pages.contains_key(&tab)
            || !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return;
        }
        let inputs = self.inputs.clone();
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::EvaluateInWorld {
                world: World::Mado,
                source: mado_engine::page::reader_probe(),
                reply: Box::new(move |r| {
                    if let Err(e) = &r {
                        tracing::debug!(%tab, error = %e, "the reader probe failed");
                    }
                    let ok = r.ok().and_then(|v| v.as_bool()).unwrap_or(false);
                    let _ = inputs.send(super::Input::Internal(Internal::ReaderProbed {
                        tab,
                        url,
                        ok,
                    }));
                }),
            },
        ));
    }

    pub(super) fn reader_probed(&mut self, tab: TabId, url: &str, ok: bool) {
        let Some(t) = self.model.tabs.get_mut(&tab) else {
            return;
        };
        if t.url != url {
            return;
        }
        t.reader_available = ok;
        let always = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|h| self.reader_always.contains(&h));
        if ok && always {
            self.open_reader(tab);
        }
    }

    fn open_reader(&mut self, tab: TabId) {
        let Some(url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) else {
            return;
        };
        let inputs = self.inputs.clone();
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::EvaluateInWorld {
                world: World::Mado,
                source: mado_engine::page::reader_extract(),
                reply: Box::new(move |r| {
                    let article = r.ok().and_then(|v| serde_json::from_value(v).ok());
                    let _ = inputs.send(super::Input::Internal(Internal::ReaderExtracted {
                        tab,
                        url,
                        article,
                    }));
                }),
            },
        ));
    }

    pub(super) fn reader_extracted(&mut self, tab: TabId, url: String, article: Option<Article>) {
        let Some(t) = self.model.tabs.get_mut(&tab) else {
            return;
        };
        if t.url != url {
            return;
        }
        let Some(article) = article else {
            t.reader_available = false;
            return;
        };
        let html = page(&article, &url, &self.look());
        self.reader_sources.insert(tab, url);
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::ShowPage {
                page: mado_engine::tab::GeneratedPage::Reader,
                html,
            },
        ));
    }

    pub(super) fn reader_command(&mut self, cmd: Command) -> Result<Value, MadoError> {
        if !self.reader_on() {
            return Err(MadoError::new(ErrorCode::Unsupported, "error.reader.off"));
        }
        match cmd {
            Command::TabReader { tab_id, on } => {
                let t = self
                    .model
                    .tabs
                    .get(&tab_id)
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?;
                match (on, t.reader_active) {
                    (true, false) => {
                        if !t.reader_available {
                            return Err(MadoError::new(
                                ErrorCode::NotFound,
                                "error.reader.notFound",
                            ));
                        }
                        self.open_reader(tab_id);
                    }
                    (false, true) => {
                        self.outbox
                            .push(EngineOp::Apply(tab_id, TabCommand::GoBack));
                    }
                    _ => {}
                }
            }
            Command::SiteSetReaderAlways { host, always } => {
                let host = host.trim().trim_start_matches("www.").to_ascii_lowercase();
                if host.is_empty() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.site.badOrigin",
                    ));
                }
                if always {
                    self.reader_always.insert(host.clone());
                    self.reader_always.insert(format!("www.{host}"));
                } else {
                    self.reader_always.remove(&host);
                    self.reader_always.remove(&format!("www.{host}"));
                }
                if !self.state.profile.is_private {
                    let pending = self.profile_db.writer().write(move |c| {
                        if always {
                            c.execute(
                                "INSERT OR REPLACE INTO reader_sites (host, always) VALUES (?1, 1)",
                                params![host],
                            )?;
                        } else {
                            c.execute("DELETE FROM reader_sites WHERE host = ?1", params![host])?;
                        }
                        Ok(())
                    });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "a Reader site was not saved");
                        }
                    });
                }
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        }
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitising_keeps_reading_and_drops_everything_else() {
        let dirty = r#"<p onclick="x()">Hi <a href="javascript:alert(1)">bad</a> <a href="https://a.example/">ok</a></p><script>alert(1)</script><img src="https://a.example/i.png" onerror="x()"><iframe src="https://evil.example"></iframe><style>body{}</style><table><tr><td colspan="2">t</td></tr></table>"#;
        let clean = sanitise(dirty);
        assert!(
            !clean.contains("script") && !clean.contains("onclick") && !clean.contains("onerror")
        );
        assert!(
            !clean.contains("javascript:")
                && !clean.contains("iframe")
                && !clean.contains("<style")
        );
        assert!(clean.contains(r#"<a href="https://a.example/" rel="noopener noreferrer">ok</a>"#));
        assert!(clean.contains(r#"<img src="https://a.example/i.png">"#));
        assert!(clean.contains(r#"<td colspan="2">t</td>"#));
    }

    #[test]
    fn the_page_escapes_its_fields_and_follows_the_settings() {
        let article = Article {
            title: "<b>T</b>".into(),
            byline: Some("By \"Q\"".into()),
            content: "<p>Body</p>".into(),
            length: 4,
            site_name: None,
        };
        let look = Look {
            font: "sans".into(),
            size: 21,
            width: "wide".into(),
            theme: "sepia".into(),
        };
        let html = page(&article, "https://news.example/a?x=1&y=2", &look);
        assert!(html.contains("<title>&lt;b&gt;T&lt;/b&gt;</title>"));
        assert!(html.contains("By &quot;Q&quot;"));
        assert!(html.contains("news.example") && html.contains("x=1&amp;y=2"));
        assert!(html.contains("21px") && html.contains("48em") && html.contains("#f4ecd8"));
        assert!(html.contains("default-src 'none'"));
    }
}
