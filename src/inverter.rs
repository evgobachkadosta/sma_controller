use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::Config;

#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    right: &'a str,
    pass: &'a str,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    result: Option<LoginResult>,
    err: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LoginResult {
    sid: String,
}

#[derive(Debug, Serialize)]
struct SetParamRequest {
    #[serde(rename = "destDev")]
    dest_dev: Vec<serde_json::Value>,
    values: Vec<serde_json::Value>,
}

/// Response from setParamValues — SMA returns {"result":{}} on success
/// or {"err":...} on failure (including session expiry → err 401 or similar)
#[derive(Debug, Deserialize)]
struct SetParamResponse {
    result: Option<serde_json::Value>,
    err: Option<serde_json::Value>,
}

/// Request body for POST /dyn/getValues.json?sid=...
/// Example: {"destDev":[],"keys":["6802_00866900"]}
#[derive(Debug, Serialize)]
struct GetValuesRequest {
    #[serde(rename = "destDev")]
    dest_dev: Vec<serde_json::Value>,
    keys: Vec<&'static str>,
}

// 6802_00866900 is the object ID for "active power limitation" on SMA inverters
const POWER_LIMIT_PARAM: &str = "6802_00866900";

const VERIFY_DELAY_SECS: u64 = 5;

pub struct InverterClient {
    config: Config,
    http: Client,
    sid: Option<String>,
}

impl InverterClient {
    pub fn new(config: Config) -> Result<Self> {
        let http = Client::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            config,
            http,
            sid: None,
        })
    }

    pub async fn login(&mut self) -> Result<()> {
        let url = format!("{}/dyn/login.json", self.config.inverter_base_url());

        info!("Logging in to inverter at {}", self.config.inverter_base_url());

        let body = LoginRequest {
            right: &self.config.inverter.right,
            pass: &self.config.inverter.password,
        };

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("HTTP request to /dyn/login.json failed")?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("Failed to read login response body")?;

        if !status.is_success() {
            bail!("Login HTTP error {}: {}", status, text);
        }

        let parsed: LoginResponse =
            serde_json::from_str(&text).with_context(|| {
                format!("Failed to parse login response JSON: {}", text)
            })?;

        if let Some(err) = parsed.err {
            bail!("Inverter returned login error: {}", err);
        }

        let sid = parsed
            .result
            .context("Login response missing 'result' field")?
            .sid;

        if sid.is_empty() {
            bail!("Login succeeded but sid is empty");
        }

        info!("Login successful, sid obtained");
        self.sid = Some(sid);
        Ok(())
    }

    pub async fn set_power_watts(&mut self, watts: u32) -> Result<()> {
        match self.set_power_inner(watts).await {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(
                    "set_power_watts({}) failed: {}. Attempting re-login",
                    watts, e
                );
                self.sid = None;
                self.login().await.context("Re-login failed")?;
                self.set_power_inner(watts)
                    .await
                    .context("set_power_watts failed even after re-login")
            }
        }
    }

    async fn set_power_inner(&mut self, watts: u32) -> Result<()> {
        let sid = self
            .sid
            .as_deref()
            .context("No active session — call login() first")?;

        let set_url = format!(
            "{}/dyn/setParamValues.json?sid={}",
            self.config.inverter_base_url(),
            sid
        );

        // Build: {"destDev":[],"values":[{"6802_00866900":{"1":[<watts>]}}]}
        let body = SetParamRequest {
            dest_dev: vec![],
            values: vec![serde_json::json!({
                POWER_LIMIT_PARAM: {
                    "1": [watts]
                }
            })],
        };

        let resp = self
            .http
            .post(&set_url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("HTTP request to setParamValues failed (watts={})", watts))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("Failed to read setParamValues response body")?;

        if !status.is_success() {
            bail!("setParamValues HTTP error {}: {}", status, text);
        }

        let parsed: SetParamResponse =
            serde_json::from_str(&text).with_context(|| {
                format!("Failed to parse setParamValues response JSON: {}", text)
            })?;

        if let Some(err) = &parsed.err {
            // SMA uses specific error codes; session expiry often shows as numeric 401/503
            bail!("Inverter returned setParamValues error: {}", err);
        }

        info!(
            "setParamValues accepted, waiting {}s before verification...",
            VERIFY_DELAY_SECS
        );
        tokio::time::sleep(std::time::Duration::from_secs(VERIFY_DELAY_SECS)).await;

        let actual = self.get_power_limit_watts().await
            .context("getValues verification call failed after setParamValues")?;

        if actual != watts {
            bail!(
                "Verification failed: sent {} W, inverter reports {} W. \
                 The command may have been silently rejected (value out of range, \
                 or inverter in a state that prevents changes).",
                watts,
                actual
            );
        }

        info!(
            "Verification passed: inverter confirmed power limit = {} W",
            actual
        );
        Ok(())
    }

    /// Read the current active power limit from the inverter via getValues.
    ///
    /// Response shape (from pysma):
    /// {
    ///   "result": {
    ///     "<device-serial>": {
    ///       "6802_00866900": {
    ///         "1": [{"val": 180000}]
    ///       }
    ///     }
    ///   }
    /// }
    async fn get_power_limit_watts(&self) -> Result<u32> {
        let sid = self
            .sid
            .as_deref()
            .context("No active session for getValues")?;

        let url = format!(
            "{}/dyn/getValues.json?sid={}",
            self.config.inverter_base_url(),
            sid
        );

        let body = GetValuesRequest {
            dest_dev: vec![],
            keys: vec![POWER_LIMIT_PARAM],
        };

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("HTTP request to getValues failed")?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("Failed to read getValues response body")?;

        if !status.is_success() {
            bail!("getValues HTTP error {}: {}", status, text);
        }

        let json: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("Failed to parse getValues JSON: {}", text))?;

        if let Some(err) = json.get("err") {
            bail!("Inverter returned getValues error: {}", err);
        }

        let result = json
            .get("result")
            .context("getValues response missing 'result' field")?
            .as_object()
            .context("getValues 'result' is not a JSON object")?;

        for (_serial, device_data) in result {
            if let Some(param_data) = device_data.get(POWER_LIMIT_PARAM) {
                let val = param_data
                    .get("1")
                    .and_then(|arr| arr.get(0))
                    .and_then(|entry| entry.get("val"))
                    .and_then(|v| v.as_u64());

                match val {
                    Some(v) => {
                        let watts = v as u32;
                        info!("getValues: {} = {} W", POWER_LIMIT_PARAM, watts);
                        return Ok(watts);
                    }
                    None => {
                        bail!(
                            "getValues: {} returned null or unexpected val shape. \
                             Raw param data: {}",
                            POWER_LIMIT_PARAM,
                            param_data
                        );
                    }
                }
            }
        }

        bail!(
            "getValues: parameter {} not found in response. Raw response: {}",
            POWER_LIMIT_PARAM,
            text
        );
    }

    /// Gracefully log out from the inverter.
    /// Errors here are non-fatal — we log them but don't propagate.
    pub async fn logout(&self) {
        if let Some(sid) = &self.sid {
            let url = format!(
                "{}/dyn/logout.json?sid={}",
                self.config.inverter_base_url(),
                sid
            );
            match self.http.post(&url).json(&serde_json::json!({})).send().await {
                Ok(_) => info!("Logged out from inverter"),
                Err(e) => warn!("Logout request failed (non-fatal): {}", e),
            }
        }
    }
}
