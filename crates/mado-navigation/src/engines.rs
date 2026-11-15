//! Search engines (spec 05 §3): the built-in list, query URLs, suggestion
//! responses and OpenSearch descriptions.

use serde::{Deserialize, Serialize};

/// A search engine, as the profile's `search_engines` table holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Engine {
    pub id: String,
    pub name: String,
    pub keyword: Option<String>,
    /// With `{searchTerms}`.
    pub search_url: String,
    /// OpenSearch JSON suggestions, with `{searchTerms}`.
    pub suggest_url: Option<String>,
}

/// The built-in engines, in the order the first-run picker lists them. None
/// is the default until the user picks one.
pub fn builtin() -> Vec<Engine> {
    let e = |id: &str, name: &str, keyword: &str, search: &str, suggest: &str| Engine {
        id: id.into(),
        name: name.into(),
        keyword: Some(keyword.into()),
        search_url: search.into(),
        suggest_url: Some(suggest.into()),
    };
    vec![
        e(
            "google",
            "Google",
            "google.com",
            "https://www.google.com/search?q={searchTerms}",
            "https://www.google.com/complete/search?client=firefox&q={searchTerms}",
        ),
        e(
            "bing",
            "Bing",
            "bing.com",
            "https://www.bing.com/search?q={searchTerms}",
            "https://api.bing.com/osjson.aspx?query={searchTerms}",
        ),
        e(
            "duckduckgo",
            "DuckDuckGo",
            "duckduckgo.com",
            "https://duckduckgo.com/?q={searchTerms}",
            "https://duckduckgo.com/ac/?q={searchTerms}&type=list",
        ),
        e(
            "brave",
            "Brave Search",
            "search.brave.com",
            "https://search.brave.com/search?q={searchTerms}",
            "https://search.brave.com/api/suggest?q={searchTerms}",
        ),
        e(
            "ecosia",
            "Ecosia",
            "ecosia.org",
            "https://www.ecosia.org/search?q={searchTerms}",
            "https://ac.ecosia.org/autocomplete?q={searchTerms}&type=list",
        ),
        e(
            "kagi",
            "Kagi",
            "kagi.com",
            "https://kagi.com/search?q={searchTerms}",
            "https://kagi.com/api/autosuggest?q={searchTerms}",
        ),
        e(
            "startpage",
            "Startpage",
            "startpage.com",
            "https://www.startpage.com/sp/search?query={searchTerms}",
            "https://www.startpage.com/osuggestions?q={searchTerms}",
        ),
        e(
            "yahoo",
            "Yahoo",
            "search.yahoo.com",
            "https://search.yahoo.com/search?p={searchTerms}",
            "https://search.yahoo.com/sugg/os?command={searchTerms}&output=fxjson",
        ),
    ]
}

/// Fills an OpenSearch template: `{searchTerms}` form-encoded, encodings
/// as UTF-8, optional parameters dropped.
pub fn expand(template: &str, terms: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(terms.as_bytes()).collect();
    let mut out = template
        .replace("{searchTerms}", &encoded)
        .replace("{inputEncoding}", "UTF-8")
        .replace("{outputEncoding}", "UTF-8");
    while let Some(start) = out.find('{') {
        let Some(len) = out[start..].find('}') else {
            break;
        };
        out.replace_range(start..=start + len, "");
    }
    out
}

/// The phrases in an OpenSearch JSON suggestion response
/// (`["query", ["one", "two"], ...]`), at most `limit`.
pub fn parse_suggestions(body: &str, limit: usize) -> Vec<String> {
    let Ok(serde_json::Value::Array(parts)) = serde_json::from_str::<serde_json::Value>(body)
    else {
        return Vec::new();
    };
    let Some(serde_json::Value::Array(list)) = parts.get(1) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 200)
        .take(limit)
        .map(str::to_owned)
        .collect()
}

/// An OpenSearch description (spec 05 §3, discovery): its name and GET
/// templates. `None` without an HTML result template.
pub fn parse_opensearch(xml: &str, host: &str) -> Option<Engine> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
    let child_text = |name: &str| {
        root.children()
            .find(|n| n.tag_name().name() == name)
            .and_then(|n| n.text())
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
    };
    let template = |kind: &str| {
        root.children()
            .filter(|n| n.tag_name().name() == "Url")
            .find(|n| {
                n.attribute("type") == Some(kind)
                    && n.attribute("method")
                        .is_none_or(|m| m.eq_ignore_ascii_case("get"))
            })
            .and_then(|n| n.attribute("template"))
            .filter(|t| t.contains("{searchTerms}") && t.starts_with("https://"))
            .map(str::to_owned)
    };
    let search_url = template("text/html")?;
    Some(Engine {
        id: format!("opensearch:{host}"),
        name: child_text("ShortName").unwrap_or_else(|| host.to_owned()),
        keyword: Some(host.to_owned()),
        search_url,
        suggest_url: template("application/x-suggestions+json"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_encode_the_terms_and_drop_optional_parameters() {
        assert_eq!(
            expand(
                "https://s.example/?q={searchTerms}&ie={inputEncoding}&p={startPage?}",
                "a b&c"
            ),
            "https://s.example/?q=a+b%26c&ie=UTF-8&p="
        );
    }

    #[test]
    fn opensearch_json_and_xml_parse() {
        assert_eq!(
            parse_suggestions(r#"["ru",["rust","rust book"],[],{}]"#, 8),
            vec!["rust", "rust book"]
        );
        assert!(parse_suggestions("not json", 8).is_empty());
        let xml = r#"<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
            <ShortName>Docs</ShortName>
            <Url type="text/html" method="get" template="https://docs.example/s?q={searchTerms}"/>
            <Url type="application/x-suggestions+json" template="https://docs.example/ac?q={searchTerms}"/>
        </OpenSearchDescription>"#;
        let e = parse_opensearch(xml, "docs.example").expect("an engine");
        assert_eq!(e.name, "Docs");
        assert_eq!(e.keyword.as_deref(), Some("docs.example"));
        assert_eq!(
            e.suggest_url.as_deref(),
            Some("https://docs.example/ac?q={searchTerms}")
        );
        let http = xml.replace("https://docs.example/s", "http://docs.example/s");
        assert!(parse_opensearch(&http, "docs.example").is_none());
    }
}
