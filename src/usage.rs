#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use std::thread;

use crate::auth;
use crate::common;
use crate::profiles;
use crate::ui;

// ─── Claude Usage API ───

#[derive(Debug, Deserialize, Serialize)]
pub struct ClaudeUsageResponse {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub seven_day_opus: Option<UsageWindow>,
    pub seven_day_sonnet: Option<UsageWindow>,
    pub iguana_necktie: Option<UsageWindow>,
    pub extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageWindow {
    pub utilization: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExtraUsage {
    pub is_enabled: Option<bool>,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
}

fn fetch_claude_usage_once(access_token: &str) -> Result<ClaudeUsageResponse> {
    let delays = [0, 2000, 4000]; // two retries on 429 with increasing backoff
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
                    anyhow::bail!("rate_limited")
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

#[derive(Debug, Deserialize, Serialize)]
pub struct CodexUsageResponse {
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub rate_limit: Option<CodexRateLimit>,
    #[serde(default)]
    pub credits: Option<CodexCredits>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CodexRateLimit {
    pub primary_window: Option<CodexWindow>,
    pub secondary_window: Option<CodexWindow>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CodexWindow {
    pub used_percent: Option<f64>,
    pub reset_at: Option<f64>, // unix timestamp seconds
    pub limit_window_seconds: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CodexCredits {
    pub has_credits: Option<bool>,
    pub unlimited: Option<bool>,
    pub balance: Option<serde_json::Value>,
}

fn fetch_codex_usage_once(access_token: &str, account_id: Option<&str>) -> Result<CodexUsageResponse> {
    let mut req = ureq::get("https://chatgpt.com/backend-api/wham/usage")
        .set("Authorization", &format!("Bearer {}", access_token))
        .set("Accept", "application/json")
        .set("User-Agent", "CodexBar");

    if let Some(aid) = account_id {
        req = req.set("ChatGPT-Account-Id", aid);
    }

    match req.call() {
        Ok(resp) => resp.into_json().context("Failed to parse Codex usage response"),
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            anyhow::bail!("token_expired")
        }
        Err(ureq::Error::Status(429, _)) => {
            anyhow::bail!("rate_limited")
        }
        Err(e) => Err(e).context("Failed to fetch Codex usage"),
    }
}

/// Try fetch, refresh token on 401/403, retry
fn fetch_codex_usage_with_refresh(
    access_token: &str,
    account_id: Option<&str>,
    refresh_token: Option<&str>,
    profile_id: Option<&str>,
) -> Result<CodexUsageResponse> {
    match fetch_codex_usage_once(access_token, account_id) {
        Ok(usage) => Ok(usage),
        Err(e) => {
            let err_str = format!("{}", e);
            if err_str.contains("token_expired") {
                if let Some(rt) = refresh_token {
                    match auth::refresh_codex_token(rt) {
                        Ok(refreshed) => {
                            // Persist refreshed tokens to profile
                            if let Some(pid) = profile_id {
                                let _ = auth::update_codex_profile_tokens(pid, &refreshed);
                            }
                            fetch_codex_usage_once(&refreshed.access_token, account_id)
                        }
                        Err(refresh_err) => anyhow::bail!(
                            "Token expired and refresh failed: {}. Re-save with `aps save codex`",
                            refresh_err
                        ),
                    }
                } else {
                    anyhow::bail!("Token expired (no refresh token). Re-save with `aps save codex`")
                }
            } else {
                Err(e)
            }
        }
    }
}

// ─── Parallel fetch infrastructure ───

/// What kind of usage to fetch
enum FetchJob {
    Claude { access_token: String, refresh_token: Option<String>, profile_id: Option<String>, is_active: bool },
    Codex { access_token: String, account_id: Option<String>, refresh_token: Option<String>, profile_id: Option<String> },
    Inactive { message: String },
    /// Section divider (not a real fetch)
    SectionHeader { title: String },
}

/// Try fetch, refresh token on 401/403, retry
fn fetch_claude_usage_with_refresh(
    access_token: &str,
    refresh_token: Option<&str>,
    profile_id: Option<&str>,
) -> Result<ClaudeUsageResponse> {
    match fetch_claude_usage_once(access_token) {
        Ok(usage) => Ok(usage),
        Err(e) => {
            let err_str = format!("{}", e);
            if err_str.contains("token_expired") {
                if let Some(rt) = refresh_token {
                    match auth::refresh_claude_token(rt) {
                        Ok(refreshed) => {
                            // Persist refreshed tokens to profile
                            if let Some(pid) = profile_id {
                                let _ = auth::update_claude_profile_tokens(pid, &refreshed);
                            }
                            fetch_claude_usage_once(&refreshed.access_token)
                        }
                        Err(refresh_err) => anyhow::bail!(
                            "Token expired and refresh failed: {}. Re-save with `aps save claude`",
                            refresh_err
                        ),
                    }
                } else {
                    anyhow::bail!("Token expired (setup token). Generate a new one with `claude setup-token`")
                }
            } else {
                Err(e)
            }
        }
    }
}

/// Result of a parallel fetch, keyed by index for ordered display
enum FetchResult {
    Claude(Result<ClaudeUsageResponse>, Option<String>),
    Codex(Result<CodexUsageResponse>, Option<String>),
    Skipped(String),
    Section(String),
}

/// Profile header info to display before usage
struct ProfileDisplay {
    tool: String,
    plan: String,
    email: String,
    label: String,
    is_active: bool,
}

// ─── Usage Cache ───

#[derive(Debug, Serialize, Deserialize)]
struct CachedUsage<T> {
    cached_at: i64, // unix timestamp seconds
    data: T,
}

fn write_usage_cache<T: Serialize>(tool: &str, profile_id: &str, data: &T) {
    let path = match common::usage_cache_path(tool, profile_id) {
        Ok(p) => p,
        Err(_) => return,
    };
    let entry = CachedUsage {
        cached_at: Utc::now().timestamp(),
        data,
    };
    if let Ok(json) = serde_json::to_vec_pretty(&entry) {
        let _ = common::atomic_write(&path, &json);
    }
}

fn read_claude_cache(profile_id: &str) -> Option<(ClaudeUsageResponse, String)> {
    let path = common::usage_cache_path("claude", profile_id).ok()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let cached: CachedUsage<ClaudeUsageResponse> = serde_json::from_str(&data).ok()?;
    let suffix = format_cache_age(cached.cached_at);
    Some((cached.data, suffix))
}

fn read_codex_cache(profile_id: &str) -> Option<(CodexUsageResponse, String)> {
    let path = common::usage_cache_path("codex", profile_id).ok()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let cached: CachedUsage<CodexUsageResponse> = serde_json::from_str(&data).ok()?;
    let suffix = format_cache_age(cached.cached_at);
    Some((cached.data, suffix))
}

fn format_cache_age(cached_at: i64) -> String {
    let now = Utc::now().timestamp();
    let age_secs = now - cached_at;
    if age_secs < 60 {
        "(cached <1m ago)".to_string()
    } else if age_secs < 3600 {
        format!("(cached {}m ago)", age_secs / 60)
    } else {
        format!("(cached {}h ago)", age_secs / 3600)
    }
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
                        let active_pid = active.map(|(id, _, _)| id.clone());
                        jobs.push((
                            ProfileDisplay { tool: "claude".into(), plan, email, label, is_active: true },
                            FetchJob::Claude { access_token: token, refresh_token, profile_id: active_pid, is_active: true },
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

                        let (email, plan, label, account_id, active_pid) = if let Some((id, meta, _)) = active {
                            (meta.email.clone(), meta.plan.clone(), meta.label.clone().unwrap_or_default(), meta.account_id.clone(), Some(id.clone()))
                        } else {
                            match auth::extract_codex_identity(&auth_data) {
                                Ok(id) => (id.email, id.plan, String::new(), Some(id.account_id), None),
                                Err(_) => ("(unknown)".into(), "?".into(), String::new(), None, None),
                            }
                        };

                        let refresh_token = auth_data.tokens.as_ref()
                            .and_then(|t| t.refresh_token.clone());
                        jobs.push((
                            ProfileDisplay { tool: "codex".into(), plan, email, label, is_active: true },
                            FetchJob::Codex { access_token: token, account_id, refresh_token, profile_id: active_pid },
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

        // Section header
        let tool_label = match tool {
            "claude" => "Claude Code",
            "codex" => "Codex",
            t => t,
        };
        jobs.push((
            ProfileDisplay {
                tool: tool.into(),
                plan: String::new(),
                email: String::new(),
                label: String::new(),
                is_active: false,
            },
            FetchJob::SectionHeader { title: tool_label.to_string() },
        ));

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
                                jobs.push((display, FetchJob::Claude { access_token: token, refresh_token, profile_id: Some(id.clone()), is_active: true }));
                                continue;
                            }
                        }
                    }
                    // Non-active Claude profiles: try with saved token + refresh.
                    // Old tokens often get rate-limited (429) after account switch.
                    let creds: auth::ClaudeCredentialsFile = serde_json::from_slice(&data)
                        .context(format!("Failed to parse profile {}", id))?;
                    let refresh_token = creds.claude_ai_oauth.as_ref()
                        .and_then(|o| o.refresh_token.clone());
                    if let Some(token) = auth::claude_access_token(&creds) {
                        jobs.push((display, FetchJob::Claude { access_token: token, refresh_token, profile_id: Some(id.clone()), is_active: *is_active }));
                    } else {
                        jobs.push((display, FetchJob::Inactive {
                            message: "No access token. Run `aps save claude` after switching to this account.".into(),
                        }));
                    }
                }
                "codex" => {
                    // For the active Codex profile, use fresh credentials from auth.json
                    if *is_active {
                        if let Ok(Some(fresh_auth)) = auth::read_codex_auth() {
                            let refresh_token = fresh_auth.tokens.as_ref()
                                .and_then(|t| t.refresh_token.clone());
                            if let Some(token) = auth::codex_access_token(&fresh_auth) {
                                let account_id = meta.account_id.clone();
                                jobs.push((display, FetchJob::Codex { access_token: token, account_id, refresh_token, profile_id: Some(id.clone()) }));
                                continue;
                            }
                        }
                    }
                    // Non-active Codex profiles: use saved token + refresh
                    let auth_data: auth::CodexAuthFile = serde_json::from_slice(&data)
                        .context(format!("Failed to parse profile {}", id))?;
                    let refresh_token = auth_data.tokens.as_ref()
                        .and_then(|t| t.refresh_token.clone());
                    if let Some(token) = auth::codex_access_token(&auth_data) {
                        let account_id = meta.account_id.clone();
                        jobs.push((display, FetchJob::Codex { access_token: token, account_id, refresh_token, profile_id: Some(id.clone()) }));
                    } else {
                        jobs.push((display, FetchJob::Codex { access_token: String::new(), account_id: None, refresh_token: None, profile_id: None }));
                    }
                }
                _ => {}
            }
        }
    }

    fetch_and_display(jobs)
}

/// Collected info for a Claude job to process sequentially
struct ClaudeJobInfo {
    idx: usize,
    access_token: String,
    refresh_token: Option<String>,
    profile_id: Option<String>,
    is_active: bool,
}

/// A collected row ready for sorting and rendering
struct CollectedRow {
    display: ProfileDisplay,
    result: FetchResult,
}

/// Fire fetches, collect ALL results, sort by weekly remaining DESC per tool section, render dashboard rows
fn fetch_and_display(jobs: Vec<(ProfileDisplay, FetchJob)>) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }

    let count = jobs.len();
    let (tx, rx) = mpsc::channel::<(usize, FetchResult)>();

    // Collect Claude jobs for sequential processing; dispatch others immediately
    let mut claude_jobs: Vec<ClaudeJobInfo> = Vec::new();

    for (idx, (_display, job)) in jobs.iter().enumerate() {
        let tx = tx.clone();
        match job {
            FetchJob::Claude { access_token, refresh_token, profile_id, is_active } => {
                if access_token.is_empty() {
                    let _ = tx.send((idx, FetchResult::Claude(Err(anyhow::anyhow!("No access token")), None)));
                    continue;
                }
                claude_jobs.push(ClaudeJobInfo {
                    idx,
                    access_token: access_token.clone(),
                    refresh_token: refresh_token.clone(),
                    profile_id: profile_id.clone(),
                    is_active: *is_active,
                });
            }
            FetchJob::Codex { access_token, account_id, refresh_token, profile_id } => {
                if access_token.is_empty() {
                    let _ = tx.send((idx, FetchResult::Codex(Err(anyhow::anyhow!("No access token")), None)));
                    continue;
                }
                let token = access_token.clone();
                let aid = account_id.clone();
                let rt = refresh_token.clone();
                let pid = profile_id.clone();
                thread::spawn(move || {
                    let result = fetch_codex_usage_with_refresh(&token, aid.as_deref(), rt.as_deref(), pid.as_deref());
                    let (fetch_result, cache_suffix) = match result {
                        Ok(usage) => {
                            if let Some(ref p) = pid {
                                write_usage_cache("codex", p, &usage);
                            }
                            (Ok(usage), None)
                        }
                        Err(e) => {
                            let err_str = format!("{}", e);
                            if err_str.contains("rate_limited") {
                                if let Some(ref p) = pid {
                                    if let Some((cached, suffix)) = read_codex_cache(p) {
                                        let _ = tx.send((idx, FetchResult::Codex(Ok(cached), Some(suffix))));
                                        return;
                                    }
                                }
                                (Err(anyhow::anyhow!("Rate limited")), None)
                            } else {
                                (Err(e), None)
                            }
                        }
                    };
                    let _ = tx.send((idx, FetchResult::Codex(fetch_result, cache_suffix)));
                });
            }
            FetchJob::Inactive { message } => {
                let msg = message.clone();
                let _ = tx.send((idx, FetchResult::Skipped(msg)));
            }
            FetchJob::SectionHeader { title } => {
                let t = title.clone();
                let _ = tx.send((idx, FetchResult::Section(t)));
            }
        }
    }

    // Sort Claude jobs: active first, then non-active
    claude_jobs.sort_by(|a, b| b.is_active.cmp(&a.is_active));

    // Spawn a single thread that processes all Claude jobs sequentially
    if !claude_jobs.is_empty() {
        let tx = tx.clone();
        thread::spawn(move || {
            for (i, job) in claude_jobs.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
                let result = fetch_claude_usage_with_refresh(
                    &job.access_token,
                    job.refresh_token.as_deref(),
                    job.profile_id.as_deref(),
                );
                let (fetch_result, cache_suffix) = match result {
                    Ok(usage) => {
                        if let Some(ref p) = job.profile_id {
                            write_usage_cache("claude", p, &usage);
                        }
                        (Ok(usage), None)
                    }
                    Err(e) => {
                        let err_str = format!("{}", e);
                        if err_str.contains("rate_limited") {
                            if let Some(ref p) = job.profile_id {
                                if let Some((cached, suffix)) = read_claude_cache(p) {
                                    let _ = tx.send((job.idx, FetchResult::Claude(Ok(cached), Some(suffix))));
                                    continue;
                                }
                            }
                            (Err(anyhow::anyhow!("Rate limited — switch to this account with `aps load claude` to view usage")), None)
                        } else if err_str.contains("token_expired") {
                            (Err(e), None)
                        } else {
                            (Err(e), None)
                        }
                    }
                };
                let _ = tx.send((job.idx, FetchResult::Claude(fetch_result, cache_suffix)));
            }
        });
    }

    drop(tx); // Close sender so rx iterator ends

    // Collect ALL results first (wait for everything)
    let mut results: Vec<Option<FetchResult>> = (0..count).map(|_| None).collect();
    for (idx, result) in rx {
        results[idx] = Some(result);
    }

    // Build collected rows, grouping by tool sections
    let mut sections: Vec<(Option<String>, Vec<CollectedRow>)> = Vec::new();
    let mut current_section_rows: Vec<CollectedRow> = Vec::new();
    let mut current_section_title: Option<String> = None;
    // Track whether we have seen any section header at all
    let mut has_sections = false;

    for (idx, result_opt) in results.into_iter().enumerate() {
        let result = result_opt.unwrap();
        let display = &jobs[idx].0;

        if let FetchResult::Section(ref title) = result {
            has_sections = true;
            // Push previous section if any
            if current_section_title.is_some() || !current_section_rows.is_empty() {
                sections.push((current_section_title.take(), std::mem::take(&mut current_section_rows)));
            }
            current_section_title = Some(title.clone());
            continue;
        }

        current_section_rows.push(CollectedRow {
            display: ProfileDisplay {
                tool: display.tool.clone(),
                plan: display.plan.clone(),
                email: display.email.clone(),
                label: display.label.clone(),
                is_active: display.is_active,
            },
            result,
        });
    }
    // Push last section
    if current_section_title.is_some() || !current_section_rows.is_empty() {
        sections.push((current_section_title, current_section_rows));
    }

    // Render each section: sort rows by weekly remaining DESC, then print via comfy-table
    for (title, mut rows) in sections {
        if let Some(ref t) = title {
            println!();
            let styled_title = match t.as_str() {
                "Claude Code" => format!("  \u{2500}\u{2500}\u{2500} {}", t.truecolor(217, 119, 87).bold()),
                "Codex" => format!("  \u{2500}\u{2500}\u{2500} {}", t.white().bold()),
                _ => format!("  \u{2500}\u{2500}\u{2500} {}", t.bold()),
            };
            println!("{}", styled_title);
        }

        // Sort by weekly remaining DESC (errors/unknown go to bottom)
        rows.sort_by(|a, b| {
            let a_pct = extract_weekly_remaining_pct(&a.display.tool, &a.result);
            let b_pct = extract_weekly_remaining_pct(&b.display.tool, &b.result);
            b_pct.cmp(&a_pct) // descending
        });

        // Add blank line before rows when no section headers
        if !rows.is_empty() && !has_sections {
            println!();
        }

        // Collect DashboardRow structs and render via table
        let dashboard_rows: Vec<ui::DashboardRow> = rows
            .iter()
            .map(|row| build_dashboard_row(&row.display, &row.result))
            .collect();

        if !dashboard_rows.is_empty() {
            let table_str = ui::build_status_table(&dashboard_rows);
            println!("{}", table_str);
        }
    }

    println!();
    Ok(())
}

/// Extract weekly remaining percentage from a fetch result (for sorting)
fn extract_weekly_remaining_pct(_tool: &str, result: &FetchResult) -> u32 {
    match result {
        FetchResult::Claude(Ok(usage), _) => {
            if let Some(ref w) = usage.seven_day {
                if let Some(util) = w.utilization {
                    return (100.0 - util.clamp(0.0, 100.0)).round() as u32;
                }
            }
            0
        }
        FetchResult::Codex(Ok(usage), _) => {
            if let Some(ref rl) = usage.rate_limit {
                if let Some(ref w) = rl.secondary_window {
                    if let Some(pct) = w.used_percent {
                        return (100.0 - pct.clamp(0.0, 100.0)).round() as u32;
                    }
                }
            }
            0
        }
        _ => 0,
    }
}

/// Build a DashboardRow from profile display info and fetch result
fn build_dashboard_row(display: &ProfileDisplay, result: &FetchResult) -> ui::DashboardRow {
    let mut row = ui::DashboardRow {
        tool: display.tool.clone(),
        plan: display.plan.clone(),
        label: display.label.clone(),
        email: display.email.clone(),
        is_active: display.is_active,
        weekly_remaining_pct: None,
        five_hour_remaining_pct: None,
        weekly_reset: String::new(),
        extra_credits: String::new(),
        cache_suffix: String::new(),
        error: String::new(),
    };

    match result {
        FetchResult::Claude(Ok(usage), cache_suffix) => {
            // Weekly (7-day) — hero metric
            if let Some(ref w) = usage.seven_day {
                if let Some(util) = w.utilization {
                    row.weekly_remaining_pct = Some((100.0 - util.clamp(0.0, 100.0)).round() as u32);
                    row.weekly_reset = format_reset_compact(w.resets_at.as_deref());
                }
            }
            // 5-hour
            if let Some(ref w) = usage.five_hour {
                if let Some(util) = w.utilization {
                    row.five_hour_remaining_pct = Some((100.0 - util.clamp(0.0, 100.0)).round() as u32);
                }
            }
            // Extra credits
            if let Some(ref extra) = usage.extra_usage {
                if extra.is_enabled == Some(true) {
                    let used_cents = extra.used_credits.unwrap_or(0.0);
                    if used_cents > 0.0 {
                        let used_dollars = used_cents / 100.0;
                        row.extra_credits = format!("+${:.0}", used_dollars);
                    }
                }
            }
            if let Some(ref suffix) = cache_suffix {
                row.cache_suffix = suffix.clone();
            }
        }
        FetchResult::Codex(Ok(usage), cache_suffix) => {
            // Weekly (secondary window) — hero metric
            if let Some(ref rl) = usage.rate_limit {
                if let Some(ref w) = rl.secondary_window {
                    if let Some(pct) = w.used_percent {
                        row.weekly_remaining_pct = Some((100.0 - pct.clamp(0.0, 100.0)).round() as u32);
                        row.weekly_reset = w.reset_at
                            .map(|ts| format_unix_reset_compact(ts as i64))
                            .unwrap_or_default();
                    }
                }
                // 5-hour (primary window)
                if let Some(ref w) = rl.primary_window {
                    if let Some(pct) = w.used_percent {
                        row.five_hour_remaining_pct = Some((100.0 - pct.clamp(0.0, 100.0)).round() as u32);
                    }
                }
            }
            if let Some(ref suffix) = cache_suffix {
                row.cache_suffix = suffix.clone();
            }
        }
        FetchResult::Claude(Err(e), _) => {
            row.error = format!("{}", e);
        }
        FetchResult::Codex(Err(e), _) => {
            row.error = format!("{}", e);
        }
        FetchResult::Skipped(msg) => {
            row.error = msg.clone();
        }
        FetchResult::Section(_) => {}
    }

    row
}

// ─── Time formatting ───

/// Compact reset time for dashboard rows: "14:00" (today) or "7 Apr" (other day)
fn format_reset_compact(iso: Option<&str>) -> String {
    if let Some(s) = iso {
        if let Ok(dt) = s.parse::<DateTime<Utc>>() {
            let local: DateTime<Local> = dt.into();
            let now = Local::now();
            if local.date_naive() == now.date_naive() {
                return local.format("%H:%M").to_string();
            } else {
                return local.format("%-d %b").to_string();
            }
        }
    }
    String::new()
}

/// Compact reset time from unix timestamp
fn format_unix_reset_compact(ts: i64) -> String {
    if let Some(dt) = DateTime::from_timestamp(ts, 0) {
        let local: DateTime<Local> = dt.into();
        let now = Local::now();
        if local.date_naive() == now.date_naive() {
            return local.format("%H:%M").to_string();
        } else {
            return local.format("%-d %b").to_string();
        }
    }
    String::new()
}

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
