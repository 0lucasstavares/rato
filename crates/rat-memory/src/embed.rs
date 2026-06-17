use reqwest::Client;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("HTTP {0}: {1}")]
    Http(u16, String),
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("bad response JSON: {0}")]
    BadJson(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct EmbeddingClient {
    base_url: String,
    key: String,
    client: Client,
}

impl EmbeddingClient {
    pub fn new(base_url: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            key: key.into(),
            client: Client::new(),
        }
    }

    /// Embed a batch of strings (≤128, each truncated to 2000 chars).
    /// Returns a Vec<Vec<f32>> of the same length.
    pub async fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let truncated: Vec<&str> = inputs
            .iter()
            .map(|s| {
                let end = s
                    .char_indices()
                    .nth(2000)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                &s[..end]
            })
            .collect();

        let body = serde_json::json!({
            "model": "text-embedding-3-small",
            "input": truncated
        });

        let url = format!("{}/v1/embeddings", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let snippet = resp.text().await.unwrap_or_default();
            let snippet = snippet.chars().take(200).collect::<String>();
            return Err(EmbedError::Http(status.as_u16(), snippet));
        }

        let val: serde_json::Value = resp.json().await?;
        let data = val["data"].as_array().ok_or_else(|| {
            EmbedError::BadJson(serde_json::from_str::<serde_json::Value>("bad").unwrap_err())
        })?;

        let mut result = Vec::with_capacity(data.len());
        for item in data {
            let emb = item["embedding"].as_array().ok_or_else(|| {
                EmbedError::BadJson(serde_json::from_str::<serde_json::Value>("bad").unwrap_err())
            })?;
            let vec: Vec<f32> = emb
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            result.push(vec);
        }
        Ok(result)
    }

    /// Embed inputs in batches of 128, collecting all results.
    pub async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut all = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(128) {
            let mut batch = self.embed_batch(chunk).await?;
            all.append(&mut batch);
        }
        Ok(all)
    }
}

/// Cosine similarity between two equal-length slices.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (&ai, &bi) in a.iter().zip(b.iter()) {
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let a = vec![1.0f32, 0.0, 0.0];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        assert!(cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![0.0f32, 0.0];
        let b = vec![1.0f32, 0.0];
        assert_eq!(cosine(&a, &b), 0.0);
    }

    #[tokio::test]
    async fn embed_batch_happy_path() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"embedding": [0.1, 0.2, 0.3]},
                    {"embedding": [0.4, 0.5, 0.6]}
                ]
            })))
            .mount(&server)
            .await;

        let client = EmbeddingClient::new(server.uri(), "test-key");
        let result = client
            .embed_batch(&["hello".to_string(), "world".to_string()])
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!((result[0][0] - 0.1).abs() < 1e-6);
    }

    #[tokio::test]
    async fn embed_truncates_long_input() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // We need to verify the request body was truncated
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"embedding": [0.1]}]
            })))
            .mount(&server)
            .await;

        let long_input = "a".repeat(5000);
        let client = EmbeddingClient::new(server.uri(), "test-key");
        let result = client.embed_batch(&[long_input]).await.unwrap();
        assert_eq!(result.len(), 1);

        // Verify the request was truncated to 2000 chars
        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        let input_str = body["input"][0].as_str().unwrap();
        assert_eq!(input_str.len(), 2000);
    }
}
