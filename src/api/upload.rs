use anyhow::{Result, Context, bail};
use reqwest::{Client, multipart};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::time::Instant;
use crate::api::roblox::*;
use crate::log;

pub struct RobloxClient {
    client: Client,
    api_key: String,
    rate_limit_reset: Mutex<Option<Instant>>,
    fatally_failed: AtomicBool,
}

impl RobloxClient {
    pub fn new(api_key: String) -> Self {
        RobloxClient {
            client: Client::new(),
            api_key,
            rate_limit_reset: Mutex::new(None),
            fatally_failed: AtomicBool::new(false),
        }
    }
    
    pub async fn upload(
        &self,
        name: &str,
        data: Vec<u8>,
        creator: Creator,
    ) -> Result<u64> {
        let request = UploadRequest {
            asset_type: "Decal".to_string(),
            display_name: name.to_string(),
            description: "Uploaded by Tungsten".to_string(),
            creation_context: CreationContext { creator },
        };
    
        let request_json = serde_json::to_string(&request)
            .context("Failed to serialize upload request")?;
    
        let data_clone = data.clone();
        let name_clone = name.to_string();
        let request_json_clone = request_json.clone();
    
        let response = self.send_with_retry(|client| {
            let form = multipart::Form::new()
                .text("request", request_json_clone.clone())
                .part(
                    "fileContent",
                    multipart::Part::bytes(data_clone.clone())
                        .file_name(name_clone.clone())
                        .mime_str("image/png")
                        .unwrap(),
                );
    
            client
                .post("https://apis.roblox.com/assets/v1/assets")
                .header("x-api-key", &self.api_key)
                .multipart(form)
        })
        .await?;
    
        let operation: Operation = response.json().await
            .context("Failed to parse upload response")?;
    
        self.poll_operation(&operation.operation_id).await
    }
    
    async fn poll_operation(&self, operation_id: &str) -> Result<u64> {
        let mut delay = Duration::from_secs(1);
        const MAX_POLLS: u32 = 10;
    
        for _attempt in 0..MAX_POLLS {
            tokio::time::sleep(delay).await;
    
            let op_id = operation_id.to_string();
            let response = self.send_with_retry(|client| {
                client
                    .get(format!("https://apis.roblox.com/assets/v1/operations/{}", op_id))
                    .header("x-api-key", &self.api_key)
            })
            .await?;
    
            let operation: Operation = response.json().await
                .context("Failed to parse operation response")?;
    
            if operation.done {
                if let Some(result) = operation.response {
                    return Ok(result.asset_id.parse()
                        .context("Failed to parse asset ID")?);
                } else {
                    bail!("Operation completed but no asset ID was returned\n  Hint: This is likely a Roblox API issue, try again");
                }
            }
    
            delay *= 2;
        }
    
        bail!(
            "Upload timed out after {} attempts\n  Hint: The asset may still be processing, check your Roblox inventory",
            MAX_POLLS
        )
    }
    
    async fn send_with_retry<F>(&self, make_req: F) -> Result<reqwest::Response>
    where
        F: Fn(&Client) -> reqwest::RequestBuilder,
    {
        if self.fatally_failed.load(Ordering::SeqCst) {
            bail!("A previous request failed fatally, aborting");
        }
    
        const MAX_RETRIES: u8 = 5;
        let mut attempt = 0;
    
        loop {
            // Wait if we're rate limited
            {
                let reset = self.rate_limit_reset.lock().await;
                if let Some(reset_at) = *reset {
                    let now = Instant::now();
                    if reset_at > now {
                        let wait = reset_at - now;
                        drop(reset);
                        tokio::time::sleep(wait).await;
                    }
                }
            }
    
            let response = make_req(&self.client).send().await
                .context("Failed to send request")?;
    
            match response.status() {
                reqwest::StatusCode::TOO_MANY_REQUESTS if attempt < MAX_RETRIES => {
                    let wait = response
                        .headers()
                        .get("x-ratelimit-reset")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| Duration::from_secs(1 << attempt));
    
                    log!(warn, "Rate limited, retrying in {:.2}s", wait.as_secs_f64());
    
                    let reset_at = Instant::now() + wait;
                    {
                        let mut reset = self.rate_limit_reset.lock().await;
                        *reset = Some(reset_at);
                    }
    
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                reqwest::StatusCode::OK => return Ok(response),
                status => {
                    let body = response.text().await.unwrap_or_default();
                    self.fatally_failed.store(true, Ordering::SeqCst);
                    bail!(
                        "Request failed with status {}\n  Response: {}\n  Hint: Check your API key and creator ID",
                        status, body
                    );
                }
            }
        }
    }
}