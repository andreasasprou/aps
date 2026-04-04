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

/// Write Claude credentials to macOS Keychain
#[cfg(target_os = "macos")]
pub fn write_claude_keychain(creds: &ClaudeCredentialsFile) -> Result<()> {
    let json_str = serde_json::to_string(creds)?;

    // Delete existing entry first (add-generic-password -U can be flaky with long values)
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-s", "Claude Code-credentials"])
        .output();

    let output = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-s", "Claude Code-credentials",
            "-a", "Claude Code-credentials",
            "-w", &json_str,
        ])
        .output()
        .context("Failed to run security command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to write to keychain: {}", stderr.trim());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn write_claude_keychain(_creds: &ClaudeCredentialsFile) -> Result<()> {
    Ok(()) // No-op on non-macOS
}

/// Get Claude credentials from file first, then keychain
pub fn read_claude_credentials() -> Result<Option<ClaudeCredentialsFile>> {
    if let Some(creds) = read_claude_credentials_file()? {
        if creds.claude_ai_oauth.is_some() {
            return Ok(Some(creds));
        }
    }
    read_claude_keychain()
}

/// Extract the access token from Claude credentials
pub fn claude_access_token(creds: &ClaudeCredentialsFile) -> Option<String> {
    creds.claude_ai_oauth.as_ref()?.access_token.clone()
}

/// Check if a Claude token is expired or will expire within 5 minutes
pub fn is_claude_token_expired(creds: &ClaudeCredentialsFile) -> bool {
    let Some(oauth) = &creds.claude_ai_oauth else {
        return true;
    };
    let Some(expires_at_ms) = oauth.expires_at else {
        // Setup tokens have no expiry and no refresh token — treat as not expired
        if oauth.refresh_token.is_none() {
            return false;
        }
        return true; // OAuth token with missing expiry = assume expired
    };
    let now_ms = chrono::Utc::now().timestamp_millis() as f64;
    let buffer_ms = 5.0 * 60.0 * 1000.0; // 5 minutes
    now_ms + buffer_ms >= expires_at_ms
}

/// Save Claude credentials to file and keychain
pub fn write_claude_credentials(creds: &ClaudeCredentialsFile) -> Result<()> {
    let path = common::claude_credentials_path()?;
    let data = serde_json::to_string_pretty(creds)?;
    // Lock ~/.claude/ during writes to avoid races with Claude Code
    let lock_path = common::claude_lock_path()?;
    let mut lock = fslock::LockFile::open(&lock_path)?;
    lock.lock().context("Failed to acquire Claude credential lock")?;
    let result = common::atomic_write(&path, data.as_bytes());
    lock.unlock().ok();
    result?;

    // Also write to keychain so Claude Code picks it up (it reads keychain first)
    if let Err(e) = write_claude_keychain(creds) {
        eprintln!("Warning: could not update keychain: {}", e);
    }
    Ok(())
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

pub fn refresh_codex_token(refresh_token: &str) -> Result<CodexTokenRefreshResponse> {
    let body = serde_json::json!({
        "client_id": CODEX_CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token
    });

    let resp = ureq::post(CODEX_REFRESH_URL)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .context("Failed to refresh Codex token")?;

    resp.into_json().context("Failed to parse refresh response")
}

#[derive(Debug, Deserialize)]
pub struct CodexTokenRefreshResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
}

pub fn refresh_claude_token(refresh_token: &str) -> Result<ClaudeTokenRefreshResponse> {
    let scope = claude_default_scopes().join(" ");
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLAUDE_CLIENT_ID,
        "scope": scope
    });

    let resp = ureq::post(CLAUDE_REFRESH_URL)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .context("Failed to refresh Claude token")?;

    resp.into_json().context("Failed to parse refresh response")
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

pub fn serialize_claude_credentials(creds: &ClaudeCredentialsFile) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(creds)?)
}

pub fn serialize_codex_auth(auth: &CodexAuthFile) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(auth)?)
}

/// Update a saved Claude profile with refreshed tokens
pub fn update_claude_profile_tokens(
    profile_id: &str,
    refreshed: &ClaudeTokenRefreshResponse,
) -> Result<()> {
    let profile_path = common::profiles_dir("claude")?.join(profile_id);
    if !profile_path.exists() {
        return Ok(());
    }

    let data = fs::read_to_string(&profile_path)?;
    let mut creds: ClaudeCredentialsFile = serde_json::from_str(&data)?;

    if let Some(ref mut oauth) = creds.claude_ai_oauth {
        oauth.access_token = Some(refreshed.access_token.clone());
        if let Some(ref new_rt) = refreshed.refresh_token {
            oauth.refresh_token = Some(new_rt.clone());
        }
        if let Some(expires_in) = refreshed.expires_in {
            oauth.expires_at = Some(
                chrono::Utc::now().timestamp_millis() as f64 + (expires_in as f64 * 1000.0),
            );
        }
    }

    let data = serde_json::to_vec_pretty(&creds)?;
    common::atomic_write(&profile_path, &data)
}

/// Update a saved Codex profile with refreshed tokens
pub fn update_codex_profile_tokens(
    profile_id: &str,
    refreshed: &CodexTokenRefreshResponse,
) -> Result<()> {
    let profile_path = common::profiles_dir("codex")?.join(profile_id);
    if !profile_path.exists() {
        return Ok(());
    }

    let data = fs::read_to_string(&profile_path)?;
    let mut auth_data: CodexAuthFile = serde_json::from_str(&data)?;

    if let Some(ref mut tokens) = auth_data.tokens {
        tokens.access_token = Some(refreshed.access_token.clone());
        if let Some(ref new_id) = refreshed.id_token {
            tokens.id_token = Some(new_id.clone());
        }
        if let Some(ref new_rt) = refreshed.refresh_token {
            tokens.refresh_token = Some(new_rt.clone());
        }
    }
    auth_data.last_refresh = Some(chrono::Utc::now().to_rfc3339());

    let data = serde_json::to_vec_pretty(&auth_data)?;
    common::atomic_write(&profile_path, &data)
}

/// Fetch Claude account info using the access token (rich metadata)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccountInfo {
    pub email_address: Option<String>,
    pub uuid: Option<String>,
    pub full_name: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub memberships: Option<Vec<ClaudeMembership>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeMembership {
    pub organization: Option<ClaudeOrganization>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeOrganization {
    pub uuid: Option<String>,
    pub name: Option<String>,
}

/// Get the default scopes for Claude Code OAuth
pub fn claude_default_scopes() -> Vec<String> {
    vec![
        "user:profile".into(),
        "user:inference".into(),
        "user:sessions:claude_code".into(),
        "user:mcp_servers".into(),
        "user:file_upload".into(),
    ]
}
