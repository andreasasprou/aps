#![allow(dead_code)]

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;

use crate::common;

// ─── Claude Code Auth ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCredentialsFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_ai_oauth: Option<ClaudeOAuth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeOAuth {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<f64>, // milliseconds since epoch
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    pub rate_limit_tier: Option<String>,
    pub subscription_type: Option<String>,
}

/// Identity info extracted from Claude credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeIdentity {
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan: Option<String>,
}

/// Read Claude credentials from ~/.claude/.credentials.json
pub fn read_claude_credentials_file() -> Result<Option<ClaudeCredentialsFile>> {
    let path = common::claude_credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(&path).context("Failed to read Claude credentials file")?;
    let creds: ClaudeCredentialsFile =
        serde_json::from_str(&data).context("Failed to parse Claude credentials")?;
    Ok(Some(creds))
}

/// Read Claude credentials from macOS Keychain
#[cfg(target_os = "macos")]
pub fn read_claude_keychain() -> Result<Option<ClaudeCredentialsFile>> {
    // Try to read from keychain using the `security` CLI tool
    // security-framework crate requires exact account name which varies per user
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let json_str = String::from_utf8(out.stdout)
                .context("Keychain data is not valid UTF-8")?;
            let creds: ClaudeCredentialsFile =
                serde_json::from_str(json_str.trim())
                    .context("Failed to parse keychain credentials")?;
            Ok(Some(creds))
        }
        _ => Ok(None),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn read_claude_keychain() -> Result<Option<ClaudeCredentialsFile>> {
    Ok(None)
}

/// Get Claude credentials from file first, then keychain
pub fn read_claude_credentials() -> Result<Option<ClaudeCredentialsFile>> {
    // Try file first
    if let Some(creds) = read_claude_credentials_file()? {
        if creds.claude_ai_oauth.is_some() {
            return Ok(Some(creds));
        }
    }
    // Fall back to keychain
    read_claude_keychain()
}

/// Extract the access token from Claude credentials
pub fn claude_access_token(creds: &ClaudeCredentialsFile) -> Option<String> {
    creds.claude_ai_oauth.as_ref()?.access_token.clone()
}

/// Try to extract identity from Claude (we don't have a JWT to decode for Claude,
/// so we'll use the API to get account info, or store what the user tells us)
pub fn extract_claude_identity(_creds: &ClaudeCredentialsFile) -> ClaudeIdentity {
    // Claude OAuth tokens don't contain email in the JWT like Codex does.
    // We'll fetch this from the API when saving.
    ClaudeIdentity {
        email: None,
        account_id: None,
        plan: None,
    }
}

/// Save Claude credentials to file
pub fn write_claude_credentials(creds: &ClaudeCredentialsFile) -> Result<()> {
    let path = common::claude_credentials_path()?;
    let data = serde_json::to_string_pretty(creds)?;
    common::atomic_write(&path, data.as_bytes())
}

// ─── Codex Auth ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthFile {
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<serde_json::Value>,
    #[serde(default)]
    pub tokens: Option<CodexTokens>,
    #[serde(default)]
    pub last_refresh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokens {
    pub id_token: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
}

/// Identity extracted from Codex JWT id_token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexIdentity {
    pub email: String,
    pub account_id: String,
    pub plan: String,
    pub plan_type_key: String,
    pub principal_id: Option<String>,
    pub workspace_or_org_id: Option<String>,
}

pub fn read_codex_auth() -> Result<Option<CodexAuthFile>> {
    let path = common::codex_auth_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(&path).context("Failed to read Codex auth file")?;
    let auth: CodexAuthFile =
        serde_json::from_str(&data).context("Failed to parse Codex auth")?;
    Ok(Some(auth))
}

pub fn write_codex_auth(auth: &CodexAuthFile) -> Result<()> {
    let path = common::codex_auth_path()?;
    let data = serde_json::to_string_pretty(auth)?;
    common::atomic_write(&path, data.as_bytes())
}

/// Decode the Codex id_token JWT to extract email and plan
pub fn extract_codex_identity(auth: &CodexAuthFile) -> Result<CodexIdentity> {
    let tokens = auth.tokens.as_ref().context("No tokens in Codex auth")?;
    let id_token = tokens
        .id_token
        .as_ref()
        .context("No id_token in Codex auth")?;

    // Decode JWT payload (second segment)
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() < 2 {
        anyhow::bail!("Invalid JWT format");
    }

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(parts[1]))
        .context("Failed to decode JWT payload")?;

    let claims: serde_json::Value =
        serde_json::from_slice(&payload).context("Failed to parse JWT claims")?;

    let email = claims
        .get("email")
        .or_else(|| claims.get("https://api.openai.com/profile").and_then(|p| p.get("email")))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let plan_type = claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_plan_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let plan_display = title_case(&plan_type);
    let account_id = tokens
        .account_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    // Try to get principal/workspace IDs from claims
    let principal_id = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(String::from);

    let workspace_or_org_id = claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("organization_id"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| Some(account_id.clone()));

    Ok(CodexIdentity {
        email,
        account_id,
        plan: plan_display,
        plan_type_key: plan_type,
        principal_id,
        workspace_or_org_id,
    })
}

/// Get Codex access token
pub fn codex_access_token(auth: &CodexAuthFile) -> Option<String> {
    auth.tokens.as_ref()?.access_token.clone()
}

// ─── Token Refresh ───

const CODEX_REFRESH_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

const CLAUDE_REFRESH_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Refresh a Codex OAuth token
pub fn refresh_codex_token(refresh_token: &str) -> Result<CodexTokenRefreshResponse> {
    let resp = ureq::post(CODEX_REFRESH_URL)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            refresh_token, CODEX_CLIENT_ID
        ))
        .context("Failed to refresh Codex token")?;

    let body: CodexTokenRefreshResponse = resp.into_json().context("Failed to parse refresh response")?;
    Ok(body)
}

#[derive(Debug, Deserialize)]
pub struct CodexTokenRefreshResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
}

/// Refresh a Claude OAuth token
pub fn refresh_claude_token(refresh_token: &str) -> Result<ClaudeTokenRefreshResponse> {
    let resp = ureq::post(CLAUDE_REFRESH_URL)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            refresh_token, CLAUDE_CLIENT_ID
        ))
        .context("Failed to refresh Claude token")?;

    let body: ClaudeTokenRefreshResponse = resp.into_json().context("Failed to parse refresh response")?;
    Ok(body)
}

#[derive(Debug, Deserialize)]
pub struct ClaudeTokenRefreshResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

// ─── Helpers ───

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Get the raw bytes of a Claude credentials for storage
pub fn serialize_claude_credentials(creds: &ClaudeCredentialsFile) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(creds)?)
}

/// Get the raw bytes of a Codex auth for storage
pub fn serialize_codex_auth(auth: &CodexAuthFile) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(auth)?)
}

/// Fetch Claude account info using the access token
pub fn fetch_claude_account_info(access_token: &str) -> Result<ClaudeAccountInfo> {
    let resp = ureq::get("https://api.anthropic.com/api/oauth/account")
        .set("Authorization", &format!("Bearer {}", access_token))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "claude-code/2.1.0")
        .set("Accept", "application/json")
        .call()
        .context("Failed to fetch Claude account info")?;

    let body: ClaudeAccountInfo = resp.into_json().context("Failed to parse Claude account info")?;
    Ok(body)
}

#[derive(Debug, Deserialize)]
pub struct ClaudeAccountInfo {
    pub email_address: Option<String>,
    pub uuid: Option<String>,
    pub full_name: Option<String>,
    pub display_name: Option<String>,
}
