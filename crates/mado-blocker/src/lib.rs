//! The built-in content blocker (spec 10): filter lists, the shared engine
//! build and its cache, and each profile's custom filters.
//!
//! Every profile shares `<root>/Blocker/`: the downloaded lists in
//! `lists/`, the resources in `resources.json`, and the serialised engine
//! in `engine-<hash>.dat`, written under `.lock` so profile processes
//! never see half a file. Building takes seconds and runs off the core
//! thread; the core keeps the engine it deserialises from the bytes.

mod build;
mod lists;
pub mod rules;

pub use adblock::Engine;
pub use adblock::resources::Resource;
pub use build::{Build, BuildError, build, custom_engine, engine_from, stored_resources};
pub use lists::{Stored, UpdateReport, client, filter_count, selected, stored_meta, update};

/// The EasyList copy bundled until the first download succeeds, so the
/// blocker works offline on first launch (10 §2 step 4). CC BY-SA 3.0,
/// © The EasyList authors.
pub fn bundled_easylist() -> String {
    use std::io::Read as _;
    let gz = include_bytes!("../data/easylist.txt.gz");
    let mut text = String::new();
    // The bundled file is checked by this crate's tests.
    let _ = flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut text);
    text
}

/// Whether `engine` blocks `url` loaded by a page at `source`, for tests
/// and the network checks of 10 §9.
pub fn blocks(engine: &adblock::Engine, url: &str, source: &str, kind: &str) -> bool {
    adblock::request::Request::new(url, source, kind, "GET")
        .is_ok_and(|r| engine.check_network_request(&r).should_block())
}

#[cfg(test)]
mod tests {
    use mado_lists::{FilterList, ListKind};

    use super::*;

    fn list(id: &str, url: &str) -> FilterList {
        FilterList {
            id: id.into(),
            url: url.into(),
            default: true,
            kind: ListKind::Ads,
        }
    }

    /// Serves `bodies` in turn, with an ETag, and answers 304 to a
    /// request carrying the last one.
    async fn serve(bodies: Vec<String>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&seen);
        tokio::spawn(async move {
            let mut n = 0usize;
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let len = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..len]).to_lowercase();
                log.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(head.clone());
                if head.contains(&format!("if-none-match: \"v{}\"", n.saturating_sub(1)))
                    && n >= bodies.len()
                {
                    let _ = s
                        .write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
                        .await;
                    continue;
                }
                let body = bodies.get(n).cloned().unwrap_or_default();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nETag: \"v{n}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes()).await;
                n += 1;
            }
        });
        (format!("http://127.0.0.1:{port}/list.txt"), seen)
    }

    fn engine(b: &Build) -> adblock::Engine {
        engine_from(&b.bytes, Vec::new()).expect("deserialises")
    }

    /// 10 §9 check 4: a cold start with no network blocks with the bundled
    /// EasyList.
    #[tokio::test(flavor = "current_thread")]
    async fn a_cold_start_without_network_blocks_with_the_bundled_easylist() {
        let dir = tempfile::tempdir().expect("dir");
        let lists = [list("easylist", "http://127.0.0.1:9/easylist.txt")];
        let report = update(&client().expect("client"), dir.path(), &lists, &[], 1).await;
        assert_eq!(report.failed, ["easylist"]);
        let b = build(dir.path(), &lists).map_err(|e| e.0).expect("builds");
        assert!(b.bundled && b.filters > 50_000, "{} filters", b.filters);
        assert!(!b.shards.is_empty(), "WebKit rules");
        let e = engine(&b);
        assert!(blocks(
            &e,
            "https://pagead2.googlesyndication.com/pagead/show_ads.js",
            "https://news.example/",
            "script"
        ));
        assert!(!blocks(
            &e,
            "https://news.example/app.js",
            "https://news.example/",
            "script"
        ));
        // A second process finds the cached engine.
        let again = build(dir.path(), &lists).map_err(|e| e.0).expect("cached");
        assert_eq!(again.hash, b.hash);
        assert_eq!(again.bytes, b.bytes);
    }

    /// 10 §9 check 2: a corrupt list download keeps the previous build.
    #[tokio::test(flavor = "current_thread")]
    async fn a_corrupt_download_keeps_the_previous_build() {
        let good: String = (0..100).map(|i| format!("||ads{i}.example^\n")).collect();
        let fewer: String = (0..20).map(|i| format!("||ads{i}.example^\n")).collect();
        let (url, seen) = serve(vec![
            good.clone(),
            "<!DOCTYPE html><html>Oops</html>".into(),
            fewer,
        ])
        .await;
        let dir = tempfile::tempdir().expect("dir");
        let lists = [list("mine", &url)];
        let c = client().expect("client");
        let report = update(&c, dir.path(), &lists, &[], 1).await;
        assert_eq!(report.changed, ["mine"]);
        let first = build(dir.path(), &lists).map_err(|e| e.0).expect("builds");
        assert_eq!(first.filters, 100);
        assert!(!first.bundled);
        assert!(blocks(
            &engine(&first),
            "https://ads7.example/x.js",
            "https://a.example/",
            "script"
        ));

        for _ in 0..2 {
            let report = update(&c, dir.path(), &lists, &[], 2).await;
            assert_eq!(
                report.rejected,
                ["mine"],
                "an HTML page, then a fifth of the filters"
            );
            assert!(report.changed.is_empty());
            let again = build(dir.path(), &lists).map_err(|e| e.0).expect("same");
            assert_eq!(again.hash, first.hash);
        }
        assert!(
            seen.lock().unwrap_or_else(|e| e.into_inner())[1].contains("if-none-match: \"v0\"")
        );

        // The build guard too: a list set that shrank to under half.
        let (other, _) = serve(vec!["||one.example^\n".into()]).await;
        let few = [list("few", &other)];
        update(&c, dir.path(), &few, &[], 3).await;
        let (err, kept) = build(dir.path(), &few).expect_err("refused");
        assert!(matches!(err, BuildError::Regression { got: 1, had: 100 }));
        assert_eq!(kept.map(|k| k.hash), Some(first.hash));
    }

    #[test]
    fn custom_filters_make_their_own_engine() {
        let e = custom_engine(
            &["||tracker.example^".into(), "! a comment".into()],
            Vec::new(),
        );
        assert!(blocks(
            &e,
            "https://tracker.example/t.gif",
            "https://a.example/",
            "image"
        ));
        assert!(!blocks(
            &e,
            "https://ok.example/t.gif",
            "https://a.example/",
            "image"
        ));
    }

    #[test]
    fn selected_lists_follow_the_setting_or_the_defaults() {
        let m = mado_lists::filter_lists();
        let defaults: Vec<String> = selected(&m, &[]).into_iter().map(|l| l.id).collect();
        assert!(defaults.contains(&"easylist".into()) && !defaults.contains(&"annoyances".into()));
        let chosen = selected(&m, &["annoyances".into()]);
        assert_eq!(chosen.len(), 1);
    }

    #[test]
    fn custom_filters_carry_procedural_filters_and_scriptlets() {
        use base64::Engine as _;
        let resource: adblock::resources::Resource = serde_json::from_value(serde_json::json!({
            "name": "mark.js", "aliases": ["mark"], "kind": "template",
            "content": base64::engine::general_purpose::STANDARD.encode("window.mark = '{{1}}';"),
        }))
        .expect("resource");
        let e = custom_engine(
            &[
                "news.example##.card:has-text(Sponsored)".into(),
                "news.example##+js(mark, ok)".into(),
            ],
            vec![resource],
        );
        let r = e.url_cosmetic_resources("https://news.example/story");
        assert!(
            r.procedural_actions.iter().any(|p| p.contains("Sponsored")),
            "{r:?}"
        );
        assert!(r.injected_script.contains("window.mark = 'ok'"), "{r:?}");
    }

    /// uBlock Origin's scriptlets as lists.yml publishes them, by hand:
    /// `MADO_UBO_RESOURCES=<file> cargo nextest run -p mado-blocker
    /// --run-ignored only`.
    #[test]
    #[ignore = "needs a generated ubo-resources.json"]
    fn ubo_scriptlets_inject() {
        let path = std::env::var("MADO_UBO_RESOURCES").expect("MADO_UBO_RESOURCES");
        let resources: Vec<adblock::resources::Resource> =
            serde_json::from_slice(&std::fs::read(path).expect("file")).expect("resources");
        let e = custom_engine(
            &["news.example##+js(set-constant, adsAllowed, true)".into()],
            resources,
        );
        let r = e.url_cosmetic_resources("https://news.example/");
        assert!(
            r.injected_script.contains("function setConstant"),
            "{}",
            &r.injected_script[..r.injected_script.len().min(300)]
        );
        assert!(r.injected_script.contains("adsAllowed"));
    }
}
