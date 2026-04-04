use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

use crate::auth;
use crate::common;
use crate::ui;

// ─── Profile Index ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileIndex {
    pub version: u32,
    pub profiles: BTreeMap<String, ProfileMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub email: String,
    pub plan: String,
    pub plan_type_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_or_org_id: Option<String>,
    // Rich metadata from Claude account API
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_tier: Option<String>,
}

impl ProfileIndex {
    fn new() -> Self {
        Self {
            version: 1,
            profiles: BTreeMap::new(),
        }
    }
}

fn load_index(tool: &str) -> Result<ProfileIndex> {
    let path = common::profiles_index_path(tool)?;
    if !path.exists() {
        return Ok(ProfileIndex::new());
    }
    let data = fs::read_to_string(&path).context("Failed to read profiles index")?;
    serde_json::from_str(&data).context("Failed to parse profiles index")
}

fn save_index(tool: &str, index: &ProfileIndex) -> Result<()> {
    let path = common::profiles_index_path(tool)?;
    let data = serde_json::to_string_pretty(index)?;
    common::atomic_write(&path, data.as_bytes())
}

fn with_lock<F, T>(tool: &str, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let lock_path = common::profiles_lock_path(tool)?;
    let mut lock = fslock::LockFile::open(&lock_path)?;
    lock.lock().context("Failed to acquire profile lock")?;
    let result = f();
    lock.unlock().context("Failed to release profile lock")?;
    result
}

// ─── Profile ID generation ───

fn claude_profile_id(email: &str, plan: &str) -> String {
    format!("{}-{}", email, plan.to_lowercase())
}

fn codex_profile_id(email: &str, plan: &str) -> String {
    format!("{}-{}", email, plan.to_lowercase())
}

// ─── Commands ───

pub fn save(tool: &str, from_token: Option<&str>, from_refresh_token: Option<&str>, label_override: Option<&str>) -> Result<()> {
    let tool = common::validate_tool(tool)?;

    match tool {
        "claude" => {
            if let Some(token) = from_token {
                save_claude_from_token(token, label_override)
            } else if let Some(rt) = from_refresh_token {
                save_claude_from_refresh_token(rt, label_override)
            } else {
                save_claude(label_override)
            }
        }
        "codex" => save_codex(label_override),
        _ => unreachable!(),
    }
}

fn save_claude(label_override: Option<&str>) -> Result<()> {
    let creds = auth::read_claude_credentials()?
        .context("No Claude credentials found. Run `claude` to authenticate first.")?;

    let oauth = creds
        .claude_ai_oauth
        .as_ref()
        .context("No OAuth credentials in Claude auth")?;

    let access_token = oauth
        .access_token
        .as_ref()
        .context("No access token in Claude credentials")?;

    // Get plan from credentials (subscriptionType field)
    let plan = oauth
        .subscription_type
        .as_deref()
        .unwrap_or("pro")
        .to_string();

    // Try to fetch account info for email + rich metadata
    print!("{}", "Fetching Claude account info... ".dimmed());
    let (email, account_id, display_name, org_name, org_uuid) =
        match auth::fetch_claude_account_info(access_token) {
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
                )
            }
            Err(e) => {
                println!("{}", "failed".yellow());
                ui::print_warning(&format!(
                    "Could not fetch account info: {}. Enter manually.",
                    e
                ));

                let email = inquire::Text::new("Email for this profile:")
                    .prompt()
                    .context("Prompt cancelled")?;
                (email, None, None, None, None)
            }
        };

    let rate_limit_tier = oauth.rate_limit_tier.clone();
    let profile_id = claude_profile_id(&email, &plan);

    // Ask for optional label (or use override from --label flag)
    let label = if let Some(l) = label_override {
        Some(l.to_string())
    } else {
        inquire::Text::new("Label (optional):")
            .with_default("")
            .prompt()
            .ok()
            .filter(|s| !s.is_empty())
    };

    with_lock("claude", || {
        let mut index = load_index("claude")?;

        if index.profiles.contains_key(&profile_id) {
            let overwrite = inquire::Confirm::new(&format!(
                "Profile '{}' already exists. Overwrite?",
                profile_id
            ))
            .with_default(false)
            .prompt()
            .unwrap_or(false);

            if !overwrite {
                println!("{}", "Save cancelled.".dimmed());
                return Ok(());
            }
        }

        let profile_path = common::profiles_dir("claude")?.join(&profile_id);
        let data = auth::serialize_claude_credentials(&creds)?;
        common::atomic_write(&profile_path, &data)?;

        index.profiles.insert(
            profile_id.clone(),
            ProfileMeta {
                email: email.clone(),
                plan: plan.clone(),
                plan_type_key: plan.to_lowercase(),
                label,
                account_id,
                principal_id: None,
                workspace_or_org_id: org_uuid.clone(),
                display_name,
                org_name: org_name.clone(),
                org_uuid,
                rate_limit_tier,
            },
        );
        save_index("claude", &index)?;
        let _ = write_stored_active_profile("claude", &profile_id);

        let org_display = org_name
            .map(|o| format!(" ({})", o))
            .unwrap_or_default();
        ui::print_success(&format!(
            "✅ Saved Claude profile: {} ({}){}",
            profile_id, email, org_display
        ));
        Ok(())
    })
}

/// Save a Claude profile from a setup token (1-year access token from `claude setup-token`)
fn save_claude_from_token(access_token: &str, label_override: Option<&str>) -> Result<()> {
    // Setup tokens only have user:inference scope — can't call account API.
    // Prompt for email directly.
    println!("{}", "Setup tokens are inference-only — enter account details:".dimmed());
    let email = inquire::Text::new("Email for this profile:")
        .prompt()
        .context("Prompt cancelled")?;
    let plan = "max".to_string();
    let (account_id, display_name, org_name, org_uuid): (Option<String>, Option<String>, Option<String>, Option<String>) =
        (None, None, None, None);

    let profile_id = claude_profile_id(&email, &plan);

    let creds = auth::ClaudeCredentialsFile {
        claude_ai_oauth: Some(auth::ClaudeOAuth {
            access_token: Some(access_token.to_string()),
            refresh_token: None,
            expires_at: None, // Claude Code treats null as "not expired"
            scopes: Some(vec!["user:inference".into()]),
            rate_limit_tier: None,
            subscription_type: Some(plan.clone()),
        }),
    };

    let label = if let Some(l) = label_override {
        Some(l.to_string())
    } else {
        inquire::Text::new("Label (optional):")
            .with_default("")
            .prompt()
            .ok()
            .filter(|s| !s.is_empty())
    };

    with_lock("claude", || {
        let mut index = load_index("claude")?;

        if index.profiles.contains_key(&profile_id) {
            let overwrite = inquire::Confirm::new(&format!(
                "Profile '{}' already exists. Overwrite?",
                profile_id
            ))
            .with_default(true)
            .prompt()
            .unwrap_or(false);

            if !overwrite {
                println!("{}", "Save cancelled.".dimmed());
                return Ok(());
            }
        }

        let profile_path = common::profiles_dir("claude")?.join(&profile_id);
        let data = auth::serialize_claude_credentials(&creds)?;
        common::atomic_write(&profile_path, &data)?;

        index.profiles.insert(
            profile_id.clone(),
            ProfileMeta {
                email: email.clone(),
                plan: plan.clone(),
                plan_type_key: plan.to_lowercase(),
                label,
                account_id,
                principal_id: None,
                workspace_or_org_id: org_uuid.clone(),
                display_name,
                org_name: org_name.clone(),
                org_uuid,
                rate_limit_tier: None,
            },
        );
        save_index("claude", &index)?;

        let org_display = org_name.map(|o| format!(" ({})", o)).unwrap_or_default();
        ui::print_success(&format!(
            "Saved Claude profile (setup token): {} ({}){}",
            profile_id, email, org_display
        ));
        Ok(())
    })
}

/// Save a Claude profile from a refresh token (skips browser login)
fn save_claude_from_refresh_token(refresh_token: &str, label_override: Option<&str>) -> Result<()> {
    print!("{}", "Refreshing token... ".dimmed());
    let refreshed = auth::refresh_claude_token(refresh_token)
        .context("Failed to refresh token. Is the refresh token valid?")?;
    println!("{}", "OK".green());

    print!("{}", "Fetching account info... ".dimmed());
    let (email, account_id, display_name, org_name, org_uuid, plan, rate_limit_tier) =
        match auth::fetch_claude_account_info(&refreshed.access_token) {
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
                ui::print_warning(&format!("Could not fetch account info: {}. Enter manually.", e));
                let email = inquire::Text::new("Email for this profile:")
                    .prompt()
                    .context("Prompt cancelled")?;
                (email, None, None, None, None, "max".to_string(), None)
            }
        };

    let profile_id = claude_profile_id(&email, &plan);
    let new_refresh_token = refreshed.refresh_token.unwrap_or_else(|| refresh_token.to_string());
    let expires_at = refreshed.expires_in.map(|secs| {
        chrono::Utc::now().timestamp_millis() as f64 + (secs as f64 * 1000.0)
    });

    let creds = auth::ClaudeCredentialsFile {
        claude_ai_oauth: Some(auth::ClaudeOAuth {
            access_token: Some(refreshed.access_token),
            refresh_token: Some(new_refresh_token),
            expires_at,
            scopes: Some(auth::claude_default_scopes()),
            rate_limit_tier: rate_limit_tier.clone(),
            subscription_type: Some(plan.clone()),
        }),
    };

    let label = if let Some(l) = label_override {
        Some(l.to_string())
    } else {
        inquire::Text::new("Label (optional):")
            .with_default("")
            .prompt()
            .ok()
            .filter(|s| !s.is_empty())
    };

    with_lock("claude", || {
        let mut index = load_index("claude")?;

        if index.profiles.contains_key(&profile_id) {
            let overwrite = inquire::Confirm::new(&format!(
                "Profile '{}' already exists. Overwrite?",
                profile_id
            ))
            .with_default(true)
            .prompt()
            .unwrap_or(false);

            if !overwrite {
                println!("{}", "Save cancelled.".dimmed());
                return Ok(());
            }
        }

        let profile_path = common::profiles_dir("claude")?.join(&profile_id);
        let data = auth::serialize_claude_credentials(&creds)?;
        common::atomic_write(&profile_path, &data)?;

        index.profiles.insert(
            profile_id.clone(),
            ProfileMeta {
                email: email.clone(),
                plan: plan.clone(),
                plan_type_key: plan.to_lowercase(),
                label,
                account_id,
                principal_id: None,
                workspace_or_org_id: org_uuid.clone(),
                display_name,
                org_name: org_name.clone(),
                org_uuid,
                rate_limit_tier,
            },
        );
        save_index("claude", &index)?;
        let _ = write_stored_active_profile("claude", &profile_id);

        let org_display = org_name.map(|o| format!(" ({})", o)).unwrap_or_default();
        ui::print_success(&format!(
            "Saved Claude profile: {} ({}){}",
            profile_id, email, org_display
        ));
        Ok(())
    })
}

fn save_codex(label_override: Option<&str>) -> Result<()> {
    let auth_data = auth::read_codex_auth()?
        .context("No Codex auth found. Run `codex` to authenticate first.")?;

    let identity = auth::extract_codex_identity(&auth_data)?;
    let profile_id = codex_profile_id(&identity.email, &identity.plan_type_key);

    // Ask for optional label (or use override from --label flag)
    let label = if let Some(l) = label_override {
        Some(l.to_string())
    } else {
        let default_label = identity.email.split('@').next().unwrap_or("").to_string();
        inquire::Text::new("Label (optional):")
            .with_default(&default_label)
            .prompt()
            .ok()
            .filter(|s| !s.is_empty())
    };

    with_lock("codex", || {
        let mut index = load_index("codex")?;

        if index.profiles.contains_key(&profile_id) {
            let overwrite = inquire::Confirm::new(&format!(
                "Profile '{}' already exists. Overwrite?",
                profile_id
            ))
            .with_default(false)
            .prompt()
            .unwrap_or(false);

            if !overwrite {
                println!("{}", "Save cancelled.".dimmed());
                return Ok(());
            }
        }

        // Save auth file
        let profile_path = common::profiles_dir("codex")?.join(&profile_id);
        let data = auth::serialize_codex_auth(&auth_data)?;
        common::atomic_write(&profile_path, &data)?;

        // Update index
        index.profiles.insert(
            profile_id.clone(),
            ProfileMeta {
                email: identity.email.clone(),
                plan: identity.plan.clone(),
                plan_type_key: identity.plan_type_key,
                label,
                account_id: Some(identity.account_id),
                principal_id: identity.principal_id,
                workspace_or_org_id: identity.workspace_or_org_id,
                display_name: None,
                org_name: None,
                org_uuid: None,
                rate_limit_tier: None,
            },
        );
        save_index("codex", &index)?;
        let _ = write_stored_active_profile("codex", &profile_id);

        ui::print_success(&format!(
            "✅ Saved Codex profile: {} ({})",
            profile_id, identity.email
        ));
        Ok(())
    })
}

pub fn load(tool: &str) -> Result<()> {
    let tool = common::validate_tool(tool)?;

    with_lock(tool, || {
        let index = load_index(tool)?;

        if index.profiles.is_empty() {
            anyhow::bail!("No saved {} profiles. Run `aps save {}` first.", tool, tool);
        }

        // Build selection list
        let active_id = get_active_profile_id(tool)?;
        let choices: Vec<String> = index
            .profiles
            .iter()
            .map(|(id, meta)| {
                let label_str = meta
                    .label
                    .as_deref()
                    .map(|l| format!(" [{}]", l))
                    .unwrap_or_default();
                let active = if Some(id.as_str()) == active_id.as_deref() {
                    " <- active"
                } else {
                    ""
                };
                format!(
                    "{} ({} - {}){}{}",
                    id, meta.email, meta.plan, label_str, active
                )
            })
            .collect();

        let selection = inquire::Select::new("Select profile to load:", choices.clone())
            .prompt()
            .context("Selection cancelled")?;

        // Find the selected profile ID
        let selected_idx = choices.iter().position(|c| c == &selection).unwrap();
        let (profile_id, _meta) = index.profiles.iter().nth(selected_idx).unwrap();

        // Load the profile
        let profile_path = common::profiles_dir(tool)?.join(profile_id);
        let data = fs::read(&profile_path)
            .context(format!("Failed to read profile file: {}", profile_id))?;

        // Track active profile for reliable detection
        let _ = write_stored_active_profile(tool, profile_id);

        match tool {
            "claude" => {
                let creds: auth::ClaudeCredentialsFile = serde_json::from_slice(&data)?;

                // Write credentials file (with file locking)
                auth::write_claude_credentials(&creds)?;
                ui::print_success(&format!("✅ Loaded Claude profile: {}", profile_id));

                // Print env var hints for fast session switching
                if let Some(ref oauth) = creds.claude_ai_oauth {
                    if let Some(ref rt) = oauth.refresh_token {
                        // Normal OAuth profile — show refresh token hint
                        let scopes = oauth
                            .scopes
                            .as_ref()
                            .map(|s| s.join(","))
                            .unwrap_or_else(|| auth::claude_default_scopes().join(","));
                        println!();
                        println!(
                            "{}",
                            "For instant switching in new shells, run:".dimmed()
                        );
                        println!(
                            "  export CLAUDE_CODE_OAUTH_REFRESH_TOKEN={}",
                            rt
                        );
                        println!(
                            "  export CLAUDE_CODE_OAUTH_SCOPES={}",
                            scopes
                        );
                    } else if let Some(ref token) = oauth.access_token {
                        // Setup token profile — show access token hint
                        println!();
                        println!(
                            "{}",
                            "For instant switching in new shells, run:".dimmed()
                        );
                        println!(
                            "  export CLAUDE_CODE_OAUTH_TOKEN={}",
                            token
                        );
                    }
                }
            }
            "codex" => {
                let auth_data: auth::CodexAuthFile = serde_json::from_slice(&data)?;
                auth::write_codex_auth(&auth_data)?;
                ui::print_success(&format!("✅ Loaded Codex profile: {}", profile_id));
            }
            _ => unreachable!(),
        }

        Ok(())
    })
}

pub fn list(tool_filter: Option<&str>) -> Result<()> {
    let tools: Vec<&str> = match tool_filter {
        Some(t) => vec![common::validate_tool(t)?],
        None => vec!["claude", "codex"],
    };

    let mut found_any = false;

    for tool in tools {
        let index = load_index(tool)?;
        if index.profiles.is_empty() {
            continue;
        }

        found_any = true;
        let active_id = get_active_profile_id(tool)?;

        println!("{}", format!("  {} profiles:", tool).bold().underline());

        for (id, meta) in &index.profiles {
            let is_active = Some(id.as_str()) == active_id.as_deref();
            let label = meta.label.as_deref().unwrap_or("");
            ui::render_profile_header_with_tool(tool, &meta.plan, &meta.email, label, is_active);
        }
    }

    if !found_any {
        println!(
            "{}",
            "No profiles saved yet. Run `aps save claude` or `aps save codex` to get started."
                .dimmed()
        );
    }

    Ok(())
}

pub fn current(tool_filter: Option<&str>) -> Result<()> {
    let tools: Vec<&str> = match tool_filter {
        Some(t) => vec![common::validate_tool(t)?],
        None => vec!["claude", "codex"],
    };

    for tool in tools {
        let index = load_index(tool)?;
        let active_id = get_active_profile_id(tool)?;

        println!("  {}:", tool.bold());
        match active_id {
            Some(ref id) => {
                if let Some(meta) = index.profiles.get(id) {
                    let label = meta.label.as_deref().unwrap_or("");
                    println!(
                        "    {} {} {}",
                        format!("[{}]", meta.plan.to_uppercase()).yellow().bold(),
                        meta.email,
                        if label.is_empty() {
                            String::new()
                        } else {
                            format!("({})", label).dimmed().to_string()
                        }
                    );
                } else {
                    println!(
                        "    Active auth detected but not a saved profile. Run `aps save {}` to save it.",
                        tool
                    );
                }
            }
            None => {
                println!("    {}", "No active auth found".dimmed());
            }
        }
    }
    Ok(())
}

pub fn delete(tool: &str) -> Result<()> {
    let tool = common::validate_tool(tool)?;

    with_lock(tool, || {
        let mut index = load_index(tool)?;

        if index.profiles.is_empty() {
            anyhow::bail!("No saved {} profiles to delete.", tool);
        }

        let choices: Vec<String> = index
            .profiles
            .iter()
            .map(|(id, meta)| {
                let label_str = meta
                    .label
                    .as_deref()
                    .map(|l| format!(" [{}]", l))
                    .unwrap_or_default();
                format!("{} ({} - {}){}", id, meta.email, meta.plan, label_str)
            })
            .collect();

        let selections = inquire::MultiSelect::new("Select profiles to delete:", choices.clone())
            .prompt()
            .context("Selection cancelled")?;

        if selections.is_empty() {
            println!("{}", "No profiles selected.".dimmed());
            return Ok(());
        }

        let confirm = inquire::Confirm::new(&format!(
            "Delete {} profile(s)?",
            selections.len()
        ))
        .with_default(false)
        .prompt()
        .unwrap_or(false);

        if !confirm {
            println!("{}", "Delete cancelled.".dimmed());
            return Ok(());
        }

        for selection in &selections {
            let idx = choices.iter().position(|c| c == selection).unwrap();
            let (profile_id, _) = index.profiles.iter().nth(idx).unwrap();
            let profile_id = profile_id.clone();

            // Delete profile file
            let profile_path = common::profiles_dir(tool)?.join(&profile_id);
            if profile_path.exists() {
                fs::remove_file(&profile_path)?;
            }

            index.profiles.remove(&profile_id);
            ui::print_success(&format!("Deleted: {}", profile_id));
        }

        save_index(tool, &index)?;
        Ok(())
    })
}

pub fn label_set(tool: &str, id: &str, label: &str) -> Result<()> {
    let tool = common::validate_tool(tool)?;
    with_lock(tool, || {
        let mut index = load_index(tool)?;
        let meta = index
            .profiles
            .get_mut(id)
            .context(format!("Profile '{}' not found", id))?;
        meta.label = Some(label.to_string());
        save_index(tool, &index)?;
        ui::print_success(&format!("Set label '{}' on profile '{}'", label, id));
        Ok(())
    })
}

pub fn label_clear(tool: &str, id: &str) -> Result<()> {
    let tool = common::validate_tool(tool)?;
    with_lock(tool, || {
        let mut index = load_index(tool)?;
        let meta = index
            .profiles
            .get_mut(id)
            .context(format!("Profile '{}' not found", id))?;
        meta.label = None;
        save_index(tool, &index)?;
        ui::print_success(&format!("Cleared label from profile '{}'", id));
        Ok(())
    })
}

pub fn label_rename(tool: &str, from: &str, to: &str) -> Result<()> {
    let tool = common::validate_tool(tool)?;
    with_lock(tool, || {
        let mut index = load_index(tool)?;
        let entry = index
            .profiles
            .values_mut()
            .find(|m| m.label.as_deref() == Some(from))
            .context(format!("No profile with label '{}' found", from))?;
        entry.label = Some(to.to_string());
        save_index(tool, &index)?;
        ui::print_success(&format!("Renamed label '{}' to '{}'", from, to));
        Ok(())
    })
}

pub fn doctor() -> Result<()> {
    println!("{}", "Running diagnostics...".bold());
    println!();

    // Check Claude
    print!("  Claude credentials: ");
    match auth::read_claude_credentials() {
        Ok(Some(creds)) => {
            if creds.claude_ai_oauth.is_some() {
                println!("{}", "✅ Found".green());

                if let Some(ref oauth) = creds.claude_ai_oauth {
                    if let Some(expires_at) = oauth.expires_at {
                        let expires = chrono::DateTime::from_timestamp_millis(expires_at as i64);
                        if let Some(dt) = expires {
                            let now = chrono::Utc::now();
                            if dt < now {
                                println!("    {}", "⚠️  Token expired".yellow());
                            } else {
                                println!(
                                    "    Expires: {}",
                                    dt.format("%Y-%m-%d %H:%M UTC").to_string().dimmed()
                                );
                            }
                        }
                    }
                }
            } else {
                println!("{}", "⚠️  File exists but no OAuth data".yellow());
            }
        }
        Ok(None) => println!("{}", "❌ Not found".red()),
        Err(e) => println!("{}", format!("❌ Error: {}", e).red()),
    }

    // Check Codex
    print!("  Codex auth: ");
    match auth::read_codex_auth() {
        Ok(Some(auth_data)) => {
            println!("{}", "✅ Found".green());
            match auth::extract_codex_identity(&auth_data) {
                Ok(identity) => {
                    println!("    Email: {}", identity.email.dimmed());
                    println!("    Plan: {}", identity.plan.dimmed());
                }
                Err(e) => println!("    {}", format!("⚠️  Could not extract identity: {}", e).yellow()),
            }
        }
        Ok(None) => println!("{}", "❌ Not found".red()),
        Err(e) => println!("{}", format!("❌ Error: {}", e).red()),
    }

    println!();

    // Check saved profiles
    for tool in common::TOOLS {
        let index = load_index(tool)?;
        println!(
            "  {} profiles saved: {}",
            tool,
            if index.profiles.is_empty() {
                "none".dimmed().to_string()
            } else {
                format!("{}", index.profiles.len()).green().to_string()
            }
        );

        for (id, meta) in &index.profiles {
            let profile_path = common::profiles_dir(tool)?.join(id);
            let exists = profile_path.exists();
            println!(
                "    {} {} ({})",
                if exists { "✅" } else { "❌" },
                id,
                meta.label.as_deref().unwrap_or("-")
            );
        }
    }

    println!();
    println!("{}", "Diagnostics complete.".bold());
    Ok(())
}

// ─── Helpers ───

/// Read the stored active profile ID (written on `aps load`)
fn read_stored_active_profile(tool: &str) -> Option<String> {
    let path = common::active_profile_path(tool).ok()?;
    fs::read_to_string(&path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Write the active profile ID to disk
fn write_stored_active_profile(tool: &str, profile_id: &str) -> Result<()> {
    let path = common::active_profile_path(tool)?;
    common::atomic_write(&path, profile_id.as_bytes())
}

/// Determine which saved profile matches the currently active auth
fn get_active_profile_id(tool: &str) -> Result<Option<String>> {
    let index = load_index(tool)?;

    match tool {
        "claude" => {
            // First: check stored active profile (set by `aps load claude`)
            if let Some(stored_id) = read_stored_active_profile("claude") {
                if index.profiles.contains_key(&stored_id) {
                    return Ok(Some(stored_id));
                }
            }

            // Fallback: match by comparing tokens
            let creds = auth::read_claude_credentials()?;
            if creds.is_none() {
                return Ok(None);
            }
            let creds = creds.unwrap();
            let current_token = auth::claude_access_token(&creds);
            if current_token.is_none() {
                return Ok(None);
            }
            let current_token = current_token.unwrap();

            for id in index.profiles.keys() {
                let profile_path = common::profiles_dir(tool)?.join(id);
                if let Ok(data) = fs::read_to_string(&profile_path) {
                    if let Ok(saved_creds) = serde_json::from_str::<auth::ClaudeCredentialsFile>(&data) {
                        if let Some(saved_token) = auth::claude_access_token(&saved_creds) {
                            if saved_token == current_token {
                                let _ = write_stored_active_profile("claude", id);
                                return Ok(Some(id.clone()));
                            }
                            let saved_refresh = saved_creds
                                .claude_ai_oauth
                                .as_ref()
                                .and_then(|o| o.refresh_token.as_ref());
                            let current_refresh = creds
                                .claude_ai_oauth
                                .as_ref()
                                .and_then(|o| o.refresh_token.as_ref());
                            if saved_refresh.is_some()
                                && saved_refresh == current_refresh
                            {
                                let _ = write_stored_active_profile("claude", id);
                                return Ok(Some(id.clone()));
                            }
                        }
                    }
                }
            }

            // Last resort: fetch account email from API and match against profile metadata
            if let Ok(info) = auth::fetch_claude_account_info(&current_token) {
                if let Some(ref email) = info.email_address {
                    for (id, meta) in &index.profiles {
                        if meta.email.eq_ignore_ascii_case(email) {
                            let _ = write_stored_active_profile("claude", id);
                            return Ok(Some(id.clone()));
                        }
                    }
                }
            }

            Ok(None)
        }
        "codex" => {
            let auth_data = auth::read_codex_auth()?;
            if auth_data.is_none() {
                return Ok(None);
            }
            let auth_data = auth_data.unwrap();
            let current_account_id = auth_data
                .tokens
                .as_ref()
                .and_then(|t| t.account_id.clone());

            if let Some(ref current_id) = current_account_id {
                for (id, meta) in &index.profiles {
                    if meta.account_id.as_deref() == Some(current_id) {
                        return Ok(Some(id.clone()));
                    }
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Get all profiles for a tool with their active status
pub fn get_all_profiles(tool: &str) -> Result<Vec<(String, ProfileMeta, bool)>> {
    let index = load_index(tool)?;
    let active_id = get_active_profile_id(tool)?;

    Ok(index
        .profiles
        .into_iter()
        .map(|(id, meta)| {
            let is_active = Some(id.as_str()) == active_id.as_deref();
            (id, meta, is_active)
        })
        .collect())
}

/// Read stored credentials for a specific profile
pub fn read_profile_credentials(tool: &str, profile_id: &str) -> Result<Vec<u8>> {
    let profile_path = common::profiles_dir(tool)?.join(profile_id);
    fs::read(&profile_path).context(format!("Failed to read profile: {}", profile_id))
}
