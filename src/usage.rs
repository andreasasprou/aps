#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use colored::Colorize;
use serde::Deserialize;
use std::sync::mpsc;
use std::thread;

use crate::auth;
use crate::profiles;
use crate::ui;

// ─── Claude Usage API ───

#[derive(Debug, Deserialize)]
pub struct ClaudeUsageResponse {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub seven_day_opus: Option<UsageWindow>,
    pub seven_day_sonnet: Option<UsageWindow>,
    pub iguana_necktie: Option<UsageWindow>,
    pub extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Deserialize)]
pub struct UsageWindow {
    pub utilization: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtraUsage {
    pub is_enabled: Option<bool>,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
}

fn fetch_claude_usage_once(access_token: &str) -> Result<ClaudeUsageResponse> {
    let delays = [0, 2000, 5000]; // retry with backoff on 429
    for (attempt, delay_ms) in delays.iter().enumerate() {
        if *delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
        }
        match ureq::get("https://api.anthropic.com/api/oauth/usage")
            .set("Authorization", &format!("Bearer {}", access_token))
            .set("anthropic-beta", "oauth-2025-04-20")
            .set("User-Agent", "claude-code/2.1.0")
            .set("Accept", "application/json")
            .call()
        {
            Ok(resp) => return resp.into_json().context("Failed to parse Claude usage response"),
            Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
                anyhow::bail!("token_expired")
            }
            Err(ureq::Error::Status(429, _)) => {
                if attempt == delays.len() - 1 {
                    anyhow::bail!("Rate limited (429). This account may be throttled by Anthropic.")
                }
                // retry
            }
            Err(ureq::Error::Status(code, _)) => {
                anyhow::bail!("HTTP {}", code)
            }
            Err(e) => return Err(e).context("Failed to fetch Claude usage"),
        }
    }
    unreachable!()
}

// ─── Codex Usage API ───

#[derive(Debug, Deserialize)]
pub struct CodexUsageResponse {
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub rate_limit: Option<CodexRateLimit>,
    #[serde(default)]
    pub credits: Option<CodexCredits>,
}

#[derive(Debug, Deserialize)]
pub struct CodexRateLimit {
    pub primary_window: Option<CodexWindow>,
    pub secondary_window: Option<CodexWindow>,
}

#[derive(Debug, Deserialize)]
pub struct CodexWindow {
    pub used_percent: Option<f64>,
    pub reset_at: Option<f64>, // unix timestamp seconds
    pub limit_window_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CodexCredits {
    pub has_credits: Option<bool>,
    pub unlimited: Option<bool>,
    pub balance: Option<serde_json::Value>,
}

fn fetch_codex_usage(access_token: &str, account_id: Option<&str>) -> Result<CodexUsageResponse> {
    let mut req = ureq::get("https://chatgpt.com/backend-api/wham/usage")
        .set("Authorization", &format!("Bearer {}", access_token))
        .set("Accept", "application/json")
        .set("User-Agent", "CodexBar");

    if let Some(aid) = account_id {
        req = req.set("ChatGPT-Account-Id", aid);
    }

    let resp = req.call().context("Failed to fetch Codex usage")?;
    resp.into_json().context("Failed to parse Codex usage response")
}

// ─── Parallel fetch infrastructure ───

/// What kind of usage to fetch
enum FetchJob {
    Claude { access_token: String, refresh_token: Option<String> },
    Codex { access_token: String, account_id: Option<String> },
}

/// Try fetch, refresh token on 401/403, retry
fn fetch_claude_usage_with_refresh(access_token: &str, refresh_token: Option<&str>) -> Result<ClaudeUsageResponse> {
    match fetch_claude_usage_once(access_token) {
        Ok(usage) => Ok(usage),
        Err(e) => {
            let err_str = format!("{}", e);
            if err_str.contains("token_expired") {
                if let Some(rt) = refresh_token {
                    match auth::refresh_claude_token(rt) {
                        Ok(refreshed) => fetch_claude_usage_once(&refreshed.access_token),
                        Err(refresh_err) => anyhow::bail!(
                            "Token expired and refresh failed: {}. Re-save with `aps save claude`",
                            refresh_err
                        ),
                    }
                } else {
                    anyhow::bail!("Token expired (no refresh token). Re-save with `aps save claude`")
                }
            } else {
                Err(e)
            }
        }
    }
}

/// Result of a parallel fetch, keyed by index for ordered display
enum FetchResult {
    Claude(Result<ClaudeUsageResponse>),
    Codex(Result<CodexUsageResponse>),
}

/// Profile header info to display before usage
struct ProfileDisplay {
    tool: String,
    plan: String,
    email: String,
    label: String,
    is_active: bool,
}

// ─── Status Command ───

pub fn status(all: bool, tool_filter: Option<&str>) -> Result<()> {
    let tools: Vec<&str> = match tool_filter {
        Some(t) => vec![crate::common::validate_tool(t)?],
        None => vec!["claude", "codex"],
    };

    if all {
        status_all_parallel(&tools)
    } else {
        status_active_parallel(&tools)
    }
}

fn status_active_parallel(tools: &[&str]) -> Result<()> {
    // Collect jobs for active profiles
    let mut jobs: Vec<(ProfileDisplay, FetchJob)> = Vec::new();

    for &tool in tools {
        match tool {
            "claude" => {
                let creds = auth::read_claude_credentials()?;
                if let Some(creds) = creds {
                    if let Some(token) = auth::claude_access_token(&creds) {
                        let all_profiles = profiles::get_all_profiles("claude")?;
                        let active = all_profiles.iter().find(|(_, _, a)| *a);

                        let (email, plan, label) = if let Some((_, meta, _)) = active {
                            (meta.email.clone(), meta.plan.clone(), meta.label.clone().unwrap_or_default())
                        } else {
                            ("(unsaved)".into(), "?".into(), String::new())
                        };

                        let refresh_token = creds.claude_ai_oauth.as_ref()
                            .and_then(|o| o.refresh_token.clone());
                        jobs.push((
                            ProfileDisplay { tool: "claude".into(), plan, email, label, is_active: true },
                            FetchJob::Claude { access_token: token, refresh_token },
                        ));
                    } else {
                        println!("  claude: {}", "No active auth".dimmed());
                    }
                } else {
                    println!("  claude: {}", "No credentials found".dimmed());
                }
            }
            "codex" => {
                let auth_data = auth::read_codex_auth()?;
                if let Some(auth_data) = auth_data {
                    if let Some(token) = auth::codex_access_token(&auth_data) {
                        let all_profiles = profiles::get_all_profiles("codex")?;
                        let active = all_profiles.iter().find(|(_, _, a)| *a);

                        let (email, plan, label, account_id) = if let Some((_, meta, _)) = active {
                            (meta.email.clone(), meta.plan.clone(), meta.label.clone().unwrap_or_default(), meta.account_id.clone())
                        } else {
                            match auth::extract_codex_identity(&auth_data) {
                                Ok(id) => (id.email, id.plan, String::new(), Some(id.account_id)),
                                Err(_) => ("(unknown)".into(), "?".into(), String::new(), None),
                            }
                        };

                        jobs.push((
                            ProfileDisplay { tool: "codex".into(), plan, email, label, is_active: true },
                            FetchJob::Codex { access_token: token, account_id },
                        ));
                    } else {
                        println!("  codex: {}", "No active auth".dimmed());
                    }
                } else {
                    println!("  codex: {}", "No auth found".dimmed());
                }
            }
            _ => {}
        }
    }

    fetch_and_display(jobs)
}

fn status_all_parallel(tools: &[&str]) -> Result<()> {
    let mut jobs: Vec<(ProfileDisplay, FetchJob)> = Vec::new();

    for &tool in tools {
        let all_profiles = profiles::get_all_profiles(tool)?;

        if all_profiles.is_empty() {
            println!("  {}: {}", tool, "No saved profiles".dimmed());
            continue;
        }

        for (id, meta, is_active) in &all_profiles {
            let label = meta.label.clone().unwrap_or_default();
            let display = ProfileDisplay {
                tool: tool.into(),
                plan: meta.plan.clone(),
                email: meta.email.clone(),
                label,
                is_active: *is_active,
            };

            let data = profiles::read_profile_credentials(tool, id)?;

            match tool {
                "claude" => {
                    // For the active Claude profile, use fresh credentials from Keychain
                    // Saved tokens get invalidated when you switch accounts
                    if *is_active {
                        if let Ok(Some(fresh_creds)) = auth::read_claude_credentials() {
                            let refresh_token = fresh_creds.claude_ai_oauth.as_ref()
                                .and_then(|o| o.refresh_token.clone());
                            if let Some(token) = auth::claude_access_token(&fresh_creds) {
                                jobs.push((display, FetchJob::Claude { access_token: token, refresh_token }));
                                continue;
                            }
                        }
                    }
                    // Non-active: use saved credentials + refresh token
                    let creds: auth::ClaudeCredentialsFile = serde_json::from_slice(&data)
                        .context(format!("Failed to parse profile {}", id))?;
                    let refresh_token = creds.claude_ai_oauth.as_ref()
                        .and_then(|o| o.refresh_token.clone());
                    if let Some(token) = auth::claude_access_token(&creds) {
                        jobs.push((display, FetchJob::Claude { access_token: token, refresh_token }));
                    } else {
                        jobs.push((display, FetchJob::Claude { access_token: String::new(), refresh_token: None }));
                    }
                }
                "codex" => {
                    let auth_data: auth::CodexAuthFile = serde_json::from_slice(&data)
                        .context(format!("Failed to parse profile {}", id))?;
                    if let Some(token) = auth::codex_access_token(&auth_data) {
                        let account_id = meta.account_id.clone();
                        jobs.push((display, FetchJob::Codex { access_token: token, account_id }));
                    } else {
                        jobs.push((display, FetchJob::Codex { access_token: String::new(), account_id: None }));
                    }
                }
                _ => {}
            }
        }
    }

    fetch_and_display(jobs)
}

/// Fire all fetches in parallel, display results in order as they arrive
fn fetch_and_display(jobs: Vec<(ProfileDisplay, FetchJob)>) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }

    let count = jobs.len();
    let (tx, rx) = mpsc::channel::<(usize, FetchResult)>();

    // Spawn all fetches — stagger Claude calls to avoid 429 rate limits
    let mut claude_idx = 0u64;
    for (idx, (_display, job)) in jobs.iter().enumerate() {
        let tx = tx.clone();
        match job {
            FetchJob::Claude { access_token, refresh_token } => {
                if access_token.is_empty() {
                    let _ = tx.send((idx, FetchResult::Claude(Err(anyhow::anyhow!("No access token")))));
                    continue;
                }
                let token = access_token.clone();
                let rt = refresh_token.clone();
                let delay_ms = claude_idx * 1000; // stagger 1s between Claude calls to avoid 429
                claude_idx += 1;
                thread::spawn(move || {
                    if delay_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    }
                    let result = fetch_claude_usage_with_refresh(&token, rt.as_deref());
                    let _ = tx.send((idx, FetchResult::Claude(result)));
                });
            }
            FetchJob::Codex { access_token, account_id } => {
                if access_token.is_empty() {
                    let _ = tx.send((idx, FetchResult::Codex(Err(anyhow::anyhow!("No access token")))));
                    continue;
                }
                let token = access_token.clone();
                let aid = account_id.clone();
                thread::spawn(move || {
                    let result = fetch_codex_usage(&token, aid.as_deref());
                    let _ = tx.send((idx, FetchResult::Codex(result)));
                });
            }
        }
    }
    drop(tx); // Close sender so rx iterator ends

    // Collect results, display in original order
    let mut results: Vec<Option<FetchResult>> = (0..count).map(|_| None).collect();
    let mut displayed_up_to = 0;

    for (idx, result) in rx {
        results[idx] = Some(result);

        // Display any contiguous results starting from where we left off
        while displayed_up_to < count {
            if results[displayed_up_to].is_none() {
                break;
            }
            let display = &jobs[displayed_up_to].0;
            let result = results[displayed_up_to].take().unwrap();

            ui::render_profile_header_with_tool(
                &display.tool,
                &display.plan,
                &display.email,
                &display.label,
                display.is_active,
            );

            match result {
                FetchResult::Claude(Ok(usage)) => print_claude_usage(&usage, 4),
                FetchResult::Claude(Err(e)) => {
                    println!("    {}", format!("Failed to fetch usage: {}", e).red());
                }
                FetchResult::Codex(Ok(usage)) => print_codex_usage(&usage, 4),
                FetchResult::Codex(Err(e)) => {
                    println!("    {}", format!("Failed to fetch usage: {}", e).red());
                }
            }

            displayed_up_to += 1;
        }
    }

    Ok(())
}

// ─── Rendering (pure, no fetching) ───

fn print_claude_usage(usage: &ClaudeUsageResponse, indent: usize) {
    // Claude API returns utilization as percentage (0-100), convert to fraction (0.0-1.0)
    if let Some(ref w) = usage.five_hour {
        if let Some(util) = w.utilization {
            let reset = format_reset_time(w.resets_at.as_deref());
            ui::render_usage_bar("5 hour", util / 100.0, &reset, indent);
        }
    }
    if let Some(ref w) = usage.seven_day {
        if let Some(util) = w.utilization {
            let reset = format_reset_time(w.resets_at.as_deref());
            ui::render_usage_bar("Weekly", util / 100.0, &reset, indent);
        }
    }
    if let Some(ref w) = usage.seven_day_opus {
        if let Some(util) = w.utilization {
            let reset = format_reset_time(w.resets_at.as_deref());
            let prefix = " ".repeat(indent);
            println!("{}{}:", prefix, "opus".bold());
            ui::render_usage_bar("  Weekly", util / 100.0, &reset, indent);
        }
    }
    if let Some(ref w) = usage.seven_day_sonnet {
        if let Some(util) = w.utilization {
            let reset = format_reset_time(w.resets_at.as_deref());
            let prefix = " ".repeat(indent);
            println!("{}{}:", prefix, "sonnet".bold());
            ui::render_usage_bar("  Weekly", util / 100.0, &reset, indent);
        }
    }
    if let Some(ref extra) = usage.extra_usage {
        if extra.is_enabled == Some(true) {
            let prefix = " ".repeat(indent);
            let currency = extra.currency.as_deref().unwrap_or("USD");
            let limit = extra.monthly_limit.unwrap_or(0.0);
            let used = extra.used_credits.unwrap_or(0.0);
            println!(
                "{}Extra credits: {:.2}/{:.2} {} used",
                prefix, used, limit, currency
            );
        }
    }
}

fn print_codex_usage(usage: &CodexUsageResponse, indent: usize) {
    if let Some(ref rl) = usage.rate_limit {
        if let Some(ref w) = rl.primary_window {
            if let Some(pct) = w.used_percent {
                let util = pct / 100.0;
                let reset = w
                    .reset_at
                    .map(|ts| format_unix_reset(ts as i64))
                    .unwrap_or_default();
                let prefix = " ".repeat(indent);
                println!("{}{}:", prefix, "codex".bold());
                ui::render_usage_bar("  5 hour", util, &reset, indent);
            }
        }
        if let Some(ref w) = rl.secondary_window {
            if let Some(pct) = w.used_percent {
                let util = pct / 100.0;
                let reset = w
                    .reset_at
                    .map(|ts| format_unix_reset(ts as i64))
                    .unwrap_or_default();
                ui::render_usage_bar("  Weekly", util, &reset, indent);
            }
        }
    }
}

// ─── Time formatting ───

fn format_reset_time(iso: Option<&str>) -> String {
    if let Some(s) = iso {
        if let Ok(dt) = s.parse::<DateTime<Utc>>() {
            let local: DateTime<Local> = dt.into();
            let now = Local::now();

            if local.date_naive() == now.date_naive() {
                return local.format("%H:%M").to_string();
            } else {
                return local.format("%H:%M on %-d %b").to_string();
            }
        }
    }
    "?".into()
}

fn format_unix_reset(ts: i64) -> String {
    if let Some(dt) = DateTime::from_timestamp(ts, 0) {
        let local: DateTime<Local> = dt.into();
        let now = Local::now();

        if local.date_naive() == now.date_naive() {
            return local.format("%H:%M").to_string();
        } else {
            return local.format("%H:%M on %-d %b").to_string();
        }
    }
    "?".into()
}
