//! Listing a connection's models (13 §2.2, §4): the connection test, and
//! the catalog's source for models `models.json` does not know.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::connection::{Api, Failure};

/// A model as the provider lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
}

/// Joins `path` to `base`, keeping the base's own path (`/v1`).
pub fn endpoint(base: &Url, path: &str) -> String {
    format!(
        "{}/{}",
        base.as_str().trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Lists the models at `base` in the `api`'s format.
pub async fn list(
    client: &reqwest::Client,
    api: Api,
    base: &Url,
    secret: Option<&str>,
) -> Result<Vec<RemoteModel>, Failure> {
    let request = match api {
        Api::Anthropic => client
            .get(endpoint(base, "v1/models?limit=1000"))
            .header("x-api-key", secret.unwrap_or_default())
            .header("anthropic-version", "2023-06-01"),
        Api::Gemini => client
            .get(endpoint(base, "v1beta/models?pageSize=1000"))
            .header("x-goog-api-key", secret.unwrap_or_default()),
        Api::OpenAiResponses | Api::OpenAiCompatible => {
            let r = client.get(endpoint(base, "models"));
            match secret.filter(|s| !s.is_empty()) {
                Some(s) => r.bearer_auth(s),
                None => r,
            }
        }
        Api::None => return Ok(Vec::new()),
    };
    let response = request.send().await.map_err(|e| {
        tracing::debug!(error = %e, "listing models failed");
        Failure::Network
    })?;
    let status = response.status().as_u16();
    let text = response.text().await.map_err(|_| Failure::Network)?;
    if !(200..300).contains(&status) {
        return Err(Failure::from_status(status, &text));
    }
    let body: Value = serde_json::from_str(&text).map_err(|_| Failure::Invalid)?;
    parse(api, &body).ok_or(Failure::Invalid)
}

/// Reads a models listing; `None` when it is not one.
pub fn parse(api: Api, body: &Value) -> Option<Vec<RemoteModel>> {
    let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    match api {
        Api::Gemini => {
            let models = body.get("models")?.as_array()?;
            Some(
                models
                    .iter()
                    .filter(|m| {
                        m["supportedGenerationMethods"]
                            .as_array()
                            .is_none_or(|a| a.iter().any(|x| x == "generateContent"))
                    })
                    .filter_map(|m| {
                        let name = text(&m["name"])?;
                        Some(RemoteModel {
                            id: name.strip_prefix("models/").unwrap_or(&name).to_owned(),
                            name: text(&m["displayName"]),
                            context_tokens: m["inputTokenLimit"].as_u64(),
                        })
                    })
                    .collect(),
            )
        }
        Api::None => Some(Vec::new()),
        _ => {
            let data = body.get("data")?.as_array()?;
            Some(
                data.iter()
                    .filter_map(|m| {
                        Some(RemoteModel {
                            id: text(&m["id"])?,
                            name: text(&m["display_name"]).or_else(|| text(&m["name"])),
                            context_tokens: m["context_length"].as_u64(),
                        })
                    })
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{Provider, base_url};
    use crate::mock::Mock;

    #[tokio::test]
    async fn each_api_lists_its_models_with_its_own_auth() {
        let mock = Mock::start(|req| {
            let key = |h: &str| req.headers.get(h).cloned().unwrap_or_default();
            match req.path.as_str() {
                "/v1/models?limit=1000" if key("x-api-key") == "ka" && key("anthropic-version") == "2023-06-01" => {
                    (200, r#"{"data":[{"id":"claude-x","display_name":"Claude X"}]}"#.into())
                }
                "/v1beta/models?pageSize=1000" if key("x-goog-api-key") == "kg" => (
                    200,
                    r#"{"models":[{"name":"models/gemini-x","displayName":"Gemini X","inputTokenLimit":1000,"supportedGenerationMethods":["generateContent"]},{"name":"models/embed","supportedGenerationMethods":["embedContent"]}]}"#.into(),
                ),
                "/v1/models" if key("authorization") == "Bearer ko" => {
                    (200, r#"{"data":[{"id":"gpt-x"},{"id":"or/y","name":"Y","context_length":8}]}"#.into())
                }
                "/local/v1/models" if key("authorization").is_empty() => {
                    (200, r#"{"data":[{"id":"llama"}]}"#.into())
                }
                "/v1/models" => (401, r#"{"error":"bad key"}"#.into()),
                _ => (404, String::new()),
            }
        })
        .await;
        let client = crate::http::client().expect("client");
        let base = |p: &str| Url::parse(&format!("{}{p}", mock.base())).expect("url");
        let ids = |r: Result<Vec<RemoteModel>, Failure>| {
            r.expect("listed")
                .into_iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(list(&client, Api::Anthropic, &base(""), Some("ka")).await),
            ["claude-x"]
        );
        let gemini = list(&client, Api::Gemini, &base(""), Some("kg"))
            .await
            .expect("listed");
        assert_eq!(
            gemini,
            [RemoteModel {
                id: "gemini-x".into(),
                name: Some("Gemini X".into()),
                context_tokens: Some(1000)
            }]
        );
        assert_eq!(
            ids(list(&client, Api::OpenAiCompatible, &base("/v1"), Some("ko")).await),
            ["gpt-x", "or/y"]
        );
        assert_eq!(
            ids(list(&client, Api::OpenAiCompatible, &base("/local/v1"), None).await),
            ["llama"]
        );
        assert_eq!(
            list(&client, Api::OpenAiResponses, &base("/v1"), Some("wrong")).await,
            Err(Failure::Unauthorized)
        );
        assert_eq!(
            list(&client, Api::OpenAiCompatible, &base("/nothing"), None).await,
            Err(Failure::NotFound)
        );
        let closed = base_url(Provider::Ollama, Some("http://127.0.0.1:9/v1")).expect("url");
        assert_eq!(
            list(&client, Api::OpenAiCompatible, &closed, None).await,
            Err(Failure::Network)
        );
    }
}
