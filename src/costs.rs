#![allow(dead_code)]

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

use crate::common;

/// Claude Code stats cache structure (~/.claude/stats-cache.json)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsCache {
    pub version: Option<u32>,
    pub last_computed_date: Option<String>,
    #[serde(default)]
    pub daily_activity: Vec<DailyActivity>,
    #[serde(default)]
    pub daily_model_tokens: Vec<DailyModelTokens>,
    #[serde(default)]
    pub model_usage: HashMap<String, ModelUsageStats>,
    pub total_sessions: Option<u64>,
    pub total_messages: Option<u64>,
    pub first_session_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    pub date: Option<String>,
    pub message_count: Option<u64>,
    pub session_count: Option<u64>,
    pub tool_call_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageStats {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub web_search_requests: u64,
    #[serde(default)]
    pub cost_usd: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyModelTokens {
    pub date: Option<String>,
    #[serde(default)]
    pub tokens_by_model: HashMap<String, u64>,
}

pub fn costs() -> Result<()> {
    println!("{}", "Claude Code Usage Stats".bold());
    println!();

    let stats_path = common::claude_stats_cache_path()?;
    if !stats_path.exists() {
        println!(
            "  {}",
            "No stats cache found. Run Claude Code to generate usage data.".dimmed()
        );
        return Ok(());
    }

    let data = fs::read_to_string(&stats_path).context("Failed to read stats cache")?;
    let stats: StatsCache =
        serde_json::from_str(&data).context("Failed to parse stats cache")?;

    // Overview
    if let Some(total) = stats.total_sessions {
        println!("  Total sessions: {}", total.to_string().green());
    }
    if let Some(total) = stats.total_messages {
        println!("  Total messages: {}", format_number(total).green());
    }
    if let Some(ref date) = stats.first_session_date {
        let short = date.split('T').next().unwrap_or(date);
        println!("  First session:  {}", short.dimmed());
    }
    println!();

    // Model usage (total tokens)
    if !stats.model_usage.is_empty() {
        println!("  {}:", "Tokens by model".bold());
        let mut models: Vec<_> = stats.model_usage.iter().collect();
        models.sort_by(|a, b| {
            let total_a = a.1.input_tokens + a.1.output_tokens;
            let total_b = b.1.input_tokens + b.1.output_tokens;
            total_b.cmp(&total_a)
        });
        for (model, usage) in &models {
            let short_name = shorten_model_name(model);
            let total = usage.input_tokens + usage.output_tokens;
            let cache = usage.cache_read_input_tokens + usage.cache_creation_input_tokens;
            println!(
                "    {:20} {:>10} tokens  {:>10} cache",
                short_name,
                format_number(total).green(),
                format_number(cache).dimmed(),
            );
        }
        println!();
    }

    // Recent daily activity (last 7 days)
    if !stats.daily_activity.is_empty() {
        println!("  {}:", "Recent activity (last 7 days)".bold());
        let recent: Vec<_> = stats
            .daily_activity
            .iter()
            .rev()
            .take(7)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        for day in &recent {
            let date = day.date.as_deref().unwrap_or("?");
            let msgs = day.message_count.unwrap_or(0);
            let sessions = day.session_count.unwrap_or(0);
            let tools = day.tool_call_count.unwrap_or(0);

            // Mini bar for messages
            let bar_len = (msgs as f64 / 5000.0 * 20.0).min(20.0) as usize;
            let bar = "█".repeat(bar_len);

            println!(
                "    {} {} {:>6} msgs  {:>3} sessions  {:>5} tools",
                date,
                bar.green(),
                format_number(msgs),
                sessions,
                format_number(tools),
            );
        }
        println!();
    }

    // Recent daily token usage
    if !stats.daily_model_tokens.is_empty() {
        println!("  {}:", "Recent tokens (last 7 days)".bold());
        let recent: Vec<_> = stats
            .daily_model_tokens
            .iter()
            .rev()
            .take(7)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        for day in &recent {
            let date = day.date.as_deref().unwrap_or("?");
            let total: u64 = day.tokens_by_model.values().sum();
            let models: Vec<_> = day
                .tokens_by_model
                .iter()
                .map(|(m, t)| format!("{}: {}", shorten_model_name(m), format_number(*t)))
                .collect();
            println!(
                "    {} {:>10} total  ({})",
                date,
                format_number(total),
                models.join(", ").dimmed()
            );
        }
    }

    Ok(())
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn shorten_model_name(model: &str) -> String {
    model
        .replace("claude-opus-4-6", "opus-4.6")
        .replace("claude-opus-4-5-20251101", "opus-4.5")
        .replace("claude-sonnet-4-6", "sonnet-4.6")
        .replace("claude-sonnet-4-5-20250929", "sonnet-4.5")
        .replace("claude-haiku-4-5-20251001", "haiku-4.5")
}
