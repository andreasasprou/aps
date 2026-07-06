#![allow(dead_code)]

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;

use crate::common;
use crate::profiles;

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
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let json_str =
                String::from_utf8(out.stdout).context("Keychain data is not valid UTF-8")?;
            let creds: ClaudeCredentialsFile = serde_json::from_str(json_str.trim())
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
            "-s",
            "Claude Code-credentials",
            "-a",
            "Claude Code-credentials",
            "-w",
            &json_str,
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
    lock.lock()
        .context("Failed to acquire Claude credential lock")?;
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
    let auth: CodexAuthFile = serde_json::from_str(&data).context("Failed to parse Codex auth")?;
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
        .or_else(|| {
            claims
                .get("https://api.openai.com/profile")
                .and_then(|p| p.get("email"))
        })
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

    let principal_id = claims.get("sub").and_then(|v| v.as_str()).map(String::from);

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
const CLAUDE_MANUAL_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const CLAUDE_MANUAL_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
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

#[derive(Debug)]
pub struct ClaudeRefreshError {
    pub status: Option<u16>,
    pub message: String,
}

impl std::fmt::Display for ClaudeRefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(code) => write!(
                f,
                "Claude token refresh failed (HTTP {}): {}",
                code, self.message
            ),
            None => write!(f, "Claude token refresh failed: {}", self.message),
        }
    }
}

impl std::error::Error for ClaudeRefreshError {}

impl ClaudeRefreshError {
    pub fn is_permanent(&self) -> bool {
        match self.status {
            Some(400 | 401) => true,
            Some(403) => {
                let msg = self.message.to_lowercase();
                msg.contains("invalid") || msg.contains("revoked")
            }
            _ => false,
        }
    }
}

pub fn refresh_claude_token(refresh_token: &str) -> Result<ClaudeTokenRefreshResponse> {
    let scope = claude_default_scopes().join(" ");
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLAUDE_CLIENT_ID,
        "scope": scope
    });

    match ureq::post(CLAUDE_REFRESH_URL)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
    {
        Ok(resp) => resp.into_json().context("Failed to parse refresh response"),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow::Error::new(ClaudeRefreshError {
                status: Some(code),
                message: body,
            }))
        }
        Err(e) => Err(anyhow::Error::new(ClaudeRefreshError {
            status: None,
            message: e.to_string(),
        })),
    }
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

pub(crate) fn apply_claude_token_refresh(
    oauth: &mut ClaudeOAuth,
    refreshed: &ClaudeTokenRefreshResponse,
) {
    oauth.access_token = Some(refreshed.access_token.clone());
    if let Some(ref new_rt) = refreshed.refresh_token {
        oauth.refresh_token = Some(new_rt.clone());
    }
    if let Some(expires_in) = refreshed.expires_in {
        oauth.expires_at =
            Some(chrono::Utc::now().timestamp_millis() as f64 + (expires_in as f64 * 1000.0));
    }
}

fn read_stored_claude_active_profile() -> Option<String> {
    let path = common::active_profile_path("claude").ok()?;
    fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Update a saved Claude profile with refreshed tokens
fn update_claude_profile_tokens(
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
        apply_claude_token_refresh(oauth, refreshed);
    }

    let data = serde_json::to_vec_pretty(&creds)?;
    common::atomic_write(&profile_path, &data)
}

/// Persist refreshed Claude tokens to the profile blob and live credentials when active
pub fn persist_refreshed_claude_tokens(
    profile_id: &str,
    refreshed: &ClaudeTokenRefreshResponse,
) -> Result<()> {
    update_claude_profile_tokens(profile_id, refreshed)?;

    if read_stored_claude_active_profile().as_deref() != Some(profile_id) {
        return Ok(());
    }

    match read_claude_credentials() {
        Ok(Some(mut live)) => {
            if let Some(ref mut oauth) = live.claude_ai_oauth {
                apply_claude_token_refresh(oauth, refreshed);
                if let Err(e) = write_claude_credentials(&live) {
                    crate::ui::print_warning(&format!(
                        "Refreshed tokens saved to profile but could not update live credentials: {}",
                        e
                    ));
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            crate::ui::print_warning(&format!(
                "Refreshed tokens saved to profile but could not read live credentials: {}",
                e
            ));
        }
    }

    Ok(())
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

    let body: ClaudeAccountInfo = resp
        .into_json()
        .context("Failed to parse Claude account info")?;
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

// ─── OAuth PKCE Flow ───

fn generate_code_verifier() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn generate_code_challenge(verifier: &str) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

fn copy_to_clipboard_osc52(text: &str) -> Result<()> {
    use std::io::Write;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    print!("\x1b]52;c;{}\x07", encoded);
    std::io::stdout()
        .flush()
        .context("Failed to flush stdout")?;
    Ok(())
}

fn start_callback_server() -> Result<tiny_http::Server> {
    // Try preferred port first, fall back to OS-assigned
    if let Ok(server) = tiny_http::Server::http("127.0.0.1:9876") {
        return Ok(server);
    }
    tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| anyhow::anyhow!("Failed to start callback server: {}", e))
}

fn wait_for_callback(server: tiny_http::Server) -> Result<(String, String)> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        // Accept one request
        if let Ok(request) = server.recv() {
            let url = request.url().to_string();
            // Send a nice HTML response back to the browser
            let response_body = "<html><body><h2>Authentication successful!</h2><p>You can close this tab and return to your terminal.</p></body></html>";
            let response = tiny_http::Response::from_string(response_body).with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap(),
            );
            let _ = request.respond(response);
            let _ = tx.send(url);
        }
    });

    let url = rx
        .recv_timeout(std::time::Duration::from_secs(300))
        .context("Timed out waiting for OAuth callback (5 minutes)")?;

    // Parse query params from /callback?code=...&state=...
    let query = url.split('?').nth(1).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        let val = kv.next().unwrap_or("");
        match key {
            "code" => code = Some(urldecode(val)),
            "state" => state = Some(urldecode(val)),
            _ => {}
        }
    }

    let code = code.context("No 'code' parameter in OAuth callback")?;
    let state = state.unwrap_or_default();
    Ok((code, state))
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

fn exchange_code_for_tokens(
    token_url: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    state: &str,
    extra_header: Option<(&str, &str)>,
) -> Result<OAuthTokenResponse> {
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": CLAUDE_CLIENT_ID,
        "code": code,
        "code_verifier": code_verifier,
        "redirect_uri": redirect_uri,
        "state": state,
    });

    let mut req = ureq::post(token_url).set("Content-Type", "application/json");
    if let Some((name, value)) = extra_header {
        req = req.set(name, value);
    }

    match req.send_string(&body.to_string()) {
        Ok(resp) => resp.into_json().context("Failed to parse token response"),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("Token exchange failed (HTTP {}): {}", code, body)
        }
        Err(e) => Err(e).context("Failed to exchange authorization code for tokens"),
    }
}

fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().unwrap_or(b'0');
            let lo = chars.next().unwrap_or(b'0');
            let hex = [hi, lo];
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&hex).unwrap_or("00"), 16) {
                result.push(val as char);
            }
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

/// Save tokens and profile after a successful Claude OAuth exchange
fn finish_claude_oauth(tokens: OAuthTokenResponse, label: Option<&str>) -> Result<()> {
    use colored::Colorize;

    // Fetch account info to get email + metadata
    print!("{}", "Fetching account info... ".dimmed());
    let (email, account_id, display_name, org_name, org_uuid, plan, rate_limit_tier) =
        match fetch_claude_account_info(&tokens.access_token) {
            Ok(info) => {
                println!("{}", "OK".green());
                let org = info
                    .memberships
                    .as_ref()
                    .and_then(|m| m.first())
                    .and_then(|m| m.organization.as_ref());
                (
                    info.email_address.unwrap_or_else(|| "unknown".into()),
                    info.uuid,
                    info.display_name,
                    org.and_then(|o| o.name.clone()),
                    org.and_then(|o| o.uuid.clone()),
                    "max".to_string(),
                    None::<String>,
                )
            }
            Err(e) => {
                println!("{}", "failed".yellow());
                crate::ui::print_warning(&format!(
                    "Could not fetch account info: {}. Enter manually.",
                    e
                ));
                let email_input = inquire::Text::new("Email for this profile:")
                    .prompt()
                    .context("Prompt cancelled")?;
                (email_input, None, None, None, None, "max".to_string(), None)
            }
        };

    let expires_at = tokens
        .expires_in
        .map(|secs| chrono::Utc::now().timestamp_millis() as f64 + (secs as f64 * 1000.0));

    let refresh_token = tokens.refresh_token.unwrap_or_default();

    let creds = ClaudeCredentialsFile {
        claude_ai_oauth: Some(ClaudeOAuth {
            access_token: Some(tokens.access_token),
            refresh_token: Some(refresh_token),
            expires_at,
            scopes: Some(claude_default_scopes()),
            rate_limit_tier: rate_limit_tier.clone(),
            subscription_type: Some(plan.clone()),
        }),
    };

    // Write credentials to the live location so Claude Code can use them immediately
    write_claude_credentials(&creds)?;

    // Save as a profile
    profiles::save_claude_oauth_profile(
        &creds,
        &email,
        &plan,
        account_id,
        display_name,
        org_name,
        org_uuid,
        rate_limit_tier,
        label,
    )?;

    Ok(())
}

/// Full OAuth PKCE flow for Claude: opens browser or manual paste, gets tokens, saves profile
pub fn oauth_claude(label: Option<&str>, manual: bool) -> Result<()> {
    use colored::Colorize;

    println!("{}", "Starting Claude OAuth authentication...".bold());
    println!();

    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = uuid::Uuid::new_v4().to_string();
    let scope = claude_default_scopes().join(" ");

    if manual {
        let auth_url = format!(
            "https://claude.ai/oauth/authorize?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
            CLAUDE_CLIENT_ID,
            urlencod(CLAUDE_MANUAL_REDIRECT_URI),
            urlencod(&scope),
            urlencod(&code_challenge),
            urlencod(&state),
        );

        println!("Open this URL in a browser to authenticate:");
        println!("  {}", auth_url);
        println!("{}", "(press c at the prompt to copy the URL)".dimmed());
        println!();

        let pasted = loop {
            let input = inquire::Text::new("Paste the authorization code (or c to copy the URL):")
                .prompt()
                .context("Prompt cancelled")?;
            let trimmed = input.trim();
            if trimmed.eq_ignore_ascii_case("c") || trimmed.eq_ignore_ascii_case("copy") {
                copy_to_clipboard_osc52(&auth_url)?;
                println!(
                    "{}",
                    "URL copied to clipboard (OSC 52 — works in most terminals; inside tmux needs set-clipboard on)."
                        .dimmed()
                );
                continue;
            }
            break input;
        };

        let mut parts = pasted.trim().splitn(2, '#');
        let code = parts
            .next()
            .context("No authorization code in pasted value")?
            .trim();
        if let Some(returned_state) = parts.next() {
            if returned_state.trim() != state {
                anyhow::bail!("OAuth state mismatch — possible CSRF attack");
            }
        }

        println!();
        print!("{}", "Exchanging code for tokens... ".dimmed());

        let tokens = exchange_code_for_tokens(
            CLAUDE_MANUAL_TOKEN_URL,
            code,
            &code_verifier,
            CLAUDE_MANUAL_REDIRECT_URI,
            &state,
            Some(("anthropic-beta", "oauth-2025-04-20")),
        )?;
        println!("{}", "OK".green());

        finish_claude_oauth(tokens, label)?;
        return Ok(());
    }

    // Start local callback server
    let server = start_callback_server()?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(9876);
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // Build authorization URL
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        "https://claude.com/cai/oauth/authorize",
        CLAUDE_CLIENT_ID,
        urlencod(&redirect_uri),
        urlencod(&scope),
        urlencod(&state),
        urlencod(&code_challenge),
    );

    println!("Opening browser for authentication...");
    println!();
    if open::that(&auth_url).is_err() {
        println!(
            "{}",
            "Could not open browser automatically. Open this URL:".yellow()
        );
        println!("  {}", auth_url);
        println!(
            "{}",
            "Or rerun with --manual on headless machines.".dimmed()
        );
    }
    println!(
        "{}",
        "Waiting for OAuth callback (up to 5 minutes)...".dimmed()
    );

    // Wait for the callback
    let (code, returned_state) = wait_for_callback(server)?;

    // Verify state
    if returned_state != state {
        anyhow::bail!("OAuth state mismatch — possible CSRF attack");
    }

    println!();
    print!("{}", "Exchanging code for tokens... ".dimmed());

    let tokens = exchange_code_for_tokens(
        CLAUDE_REFRESH_URL,
        &code,
        &code_verifier,
        &redirect_uri,
        &state,
        None,
    )?;
    println!("{}", "OK".green());

    finish_claude_oauth(tokens, label)?;

    Ok(())
}

/// Minimal percent-encoding for URL query parameter values
fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

// ─── Codex OAuth PKCE Flow ───

const CODEX_ISSUER: &str = "https://auth.openai.com";
const CODEX_OAUTH_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";

#[derive(Debug, Deserialize)]
struct CodexOAuthTokenResponse {
    id_token: String,
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct CodexDeviceUserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "user_code", alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Option<serde_json::Value>,
}

impl CodexDeviceUserCodeResponse {
    fn poll_interval_secs(&self) -> u64 {
        match self.interval.as_ref() {
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(5).max(1),
            Some(serde_json::Value::String(s)) => s.trim().parse::<u64>().unwrap_or(5).max(1),
            _ => 5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CodexDeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

fn exchange_codex_authorization_code(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<CodexOAuthTokenResponse> {
    // Codex uses form-urlencoded for token exchange (not JSON).
    let token_body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        urlencod(code),
        urlencod(redirect_uri),
        urlencod(CODEX_CLIENT_ID),
        urlencod(code_verifier),
    );

    let token_resp = match ureq::post(&format!("{}/oauth/token", CODEX_ISSUER))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&token_body)
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("Token exchange failed (HTTP {}): {}", code, body);
        }
        Err(e) => return Err(e).context("Failed to exchange authorization code"),
    };

    token_resp
        .into_json()
        .context("Failed to parse Codex token response")
}

fn finish_codex_oauth(tokens: CodexOAuthTokenResponse, label: Option<&str>) -> Result<()> {
    use colored::Colorize;

    let auth_data = CodexAuthFile {
        auth_mode: Some("chatgpt".into()),
        openai_api_key: None,
        tokens: Some(CodexTokens {
            id_token: Some(tokens.id_token),
            access_token: Some(tokens.access_token),
            refresh_token: Some(tokens.refresh_token),
            account_id: None,
        }),
        last_refresh: Some(chrono::Utc::now().to_rfc3339()),
    };

    let identity = extract_codex_identity(&auth_data)?;

    println!(
        "{}",
        format!("  Account: {} ({})", identity.email, identity.plan).dimmed()
    );

    profiles::save_codex_oauth_profile(auth_data, &identity, label)
}

fn codex_device_request_user_code() -> Result<CodexDeviceUserCodeResponse> {
    let url = format!("{}/api/accounts/deviceauth/usercode", CODEX_ISSUER);
    let body = serde_json::json!({
        "client_id": CODEX_CLIENT_ID,
    });

    match ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
    {
        Ok(resp) => resp
            .into_json()
            .context("Failed to parse Codex device-code response"),
        Err(ureq::Error::Status(404, _)) => {
            anyhow::bail!(
                "Codex device auth is not enabled for this server/account. Use browser login or enable device code authentication."
            )
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            if body.trim().is_empty() {
                anyhow::bail!("Codex device-code request failed (HTTP {})", code);
            }
            anyhow::bail!("Codex device-code request failed (HTTP {}): {}", code, body);
        }
        Err(e) => Err(e).context("Failed to request Codex device code"),
    }
}

fn codex_device_poll_for_token(
    device_auth_id: &str,
    user_code: &str,
    interval_secs: u64,
) -> Result<CodexDeviceTokenResponse> {
    let url = format!("{}/api/accounts/deviceauth/token", CODEX_ISSUER);
    let max_wait = std::time::Duration::from_secs(15 * 60);
    let start = std::time::Instant::now();

    loop {
        let body = serde_json::json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        });

        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
        {
            Ok(resp) => {
                return resp
                    .into_json()
                    .context("Failed to parse Codex device-token response");
            }
            Err(ureq::Error::Status(403 | 404, _)) => {
                let elapsed = start.elapsed();
                if elapsed >= max_wait {
                    anyhow::bail!("Codex device auth timed out after 15 minutes");
                }
                let sleep_for =
                    std::time::Duration::from_secs(interval_secs).min(max_wait - elapsed);
                std::thread::sleep(sleep_for);
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if body.trim().is_empty() {
                    anyhow::bail!("Codex device auth failed (HTTP {})", code);
                }
                anyhow::bail!("Codex device auth failed (HTTP {}): {}", code, body);
            }
            Err(e) => return Err(e).context("Failed while polling Codex device auth"),
        }
    }
}

fn oauth_codex_device(label: Option<&str>) -> Result<()> {
    use colored::Colorize;

    println!("{}", "Starting Codex device authentication...".bold());
    println!();

    print!("{}", "Requesting device code... ".dimmed());
    let device_code = codex_device_request_user_code()?;
    println!("{}", "OK".green());
    println!();

    println!("Open this URL in a browser and sign in:");
    println!("  {}", format!("{}/codex/device", CODEX_ISSUER).blue());
    println!();
    println!("Enter this one-time code:");
    println!("  {}", device_code.user_code.blue().bold());
    println!();
    println!(
        "{}",
        "Device codes expire in 15 minutes. Never share this code.".dimmed()
    );
    println!(
        "{}",
        "Waiting for device authorization (up to 15 minutes)...".dimmed()
    );

    let code_resp = codex_device_poll_for_token(
        &device_code.device_auth_id,
        &device_code.user_code,
        device_code.poll_interval_secs(),
    )?;

    println!();
    print!("{}", "Exchanging code for tokens... ".dimmed());
    let redirect_uri = format!("{}/deviceauth/callback", CODEX_ISSUER);
    let tokens = exchange_codex_authorization_code(
        &code_resp.authorization_code,
        &code_resp.code_verifier,
        &redirect_uri,
    )?;
    println!("{}", "OK".green());

    finish_codex_oauth(tokens, label)
}

/// Full OAuth flow for Codex: browser callback or device code, gets tokens, saves profile.
pub fn oauth_codex(label: Option<&str>, device_auth: bool) -> Result<()> {
    if device_auth {
        return oauth_codex_device(label);
    }

    oauth_codex_browser(label)
}

fn oauth_codex_browser(label: Option<&str>) -> Result<()> {
    use colored::Colorize;

    println!("{}", "Starting Codex OAuth authentication...".bold());
    println!();

    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = uuid::Uuid::new_v4().to_string();

    // Start local callback server (Codex uses port 1455 by default)
    let server = tiny_http::Server::http("localhost:1455")
        .or_else(|_| tiny_http::Server::http("localhost:0"))
        .map_err(|e| anyhow::anyhow!("Failed to start callback server: {}", e))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(1455);
    let redirect_uri = format!("http://localhost:{}/auth/callback", port);

    // Build authorization URL (matching Codex CLI's exact parameters)
    let auth_url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=codex_cli_rs",
        CODEX_ISSUER,
        urlencod(CODEX_CLIENT_ID),
        urlencod(&redirect_uri),
        urlencod(CODEX_OAUTH_SCOPES),
        urlencod(&code_challenge),
        urlencod(&state),
    );

    println!("Opening browser for authentication...");
    println!();
    if open::that(&auth_url).is_err() {
        println!(
            "{}",
            "Could not open browser automatically. Open this URL:".yellow()
        );
        println!("  {}", auth_url);
    }
    println!(
        "{}",
        "Waiting for OAuth callback (up to 5 minutes)...".dimmed()
    );

    // Wait for callback
    let (code, returned_state) = wait_for_callback(server)?;

    if returned_state != state {
        anyhow::bail!("OAuth state mismatch — possible CSRF attack");
    }

    println!();
    print!("{}", "Exchanging code for tokens... ".dimmed());
    let tokens = exchange_codex_authorization_code(&code, &code_verifier, &redirect_uri)?;
    println!("{}", "OK".green());

    finish_codex_oauth(tokens, label)
}
