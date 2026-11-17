//! Local servers (13 §2.2): Ollama and LM Studio on this Mac, probed each
//! time the AI settings open.

use std::time::Duration;

use crate::connection::{Api, Provider, base_url};

/// One probe waits this long; a local server answers at once or is off.
const PROBE: Duration = Duration::from_millis(800);

/// A server that answered, with how many models it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub provider: Provider,
    pub base_url: String,
    pub models: usize,
}

/// Probes each `(provider, base)`; the app passes Ollama's and LM Studio's
/// default addresses (tests pass mocks).
pub async fn detect(client: &reqwest::Client, candidates: &[(Provider, String)]) -> Vec<Found> {
    let mut found = Vec::new();
    for (provider, base) in candidates {
        let Ok(url) = base_url(*provider, Some(base)) else {
            continue;
        };
        let listed = tokio::time::timeout(
            PROBE,
            crate::models::list(client, Api::OpenAiCompatible, &url, None),
        )
        .await;
        if let Ok(Ok(models)) = listed {
            found.push(Found {
                provider: *provider,
                base_url: base.clone(),
                models: models.len(),
            });
        }
    }
    found
}

/// Where Ollama and LM Studio listen by default.
pub fn defaults() -> Vec<(Provider, String)> {
    [Provider::Ollama, Provider::LmStudio]
        .into_iter()
        .filter_map(|p| Some((p, p.default_base()?.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Mock;

    #[tokio::test]
    async fn only_servers_that_answer_are_offered() {
        let mock = Mock::start(|req| match req.path.as_str() {
            "/v1/models" => (200, r#"{"data":[{"id":"a"},{"id":"b"}]}"#.into()),
            _ => (404, String::new()),
        })
        .await;
        let client = crate::http::client().expect("client");
        let found = detect(
            &client,
            &[
                (Provider::Ollama, format!("{}/v1", mock.base())),
                (Provider::LmStudio, "http://127.0.0.1:9/v1".into()),
            ],
        )
        .await;
        assert_eq!(
            found,
            [Found {
                provider: Provider::Ollama,
                base_url: format!("{}/v1", mock.base()),
                models: 2
            }]
        );
        assert_eq!(defaults().len(), 2);
    }
}
