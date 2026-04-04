#![allow(dead_code)]

use colored::Colorize;

pub fn print_error(msg: &str) {
    eprintln!("{} {}", "[ERROR]".red().bold(), msg);
}

pub fn print_success(msg: &str) {
    println!("{}", msg.green());
}

pub fn print_warning(msg: &str) {
    println!("{} {}", "⚠️".yellow(), msg.yellow());
}

pub fn print_hint(msg: &str) {
    println!("  {}", msg.dimmed());
}

/// Render a usage progress bar like codex-profiles
/// utilization is 0.0 to 1.0 (fraction used)
/// Bar shows: █ = remaining (colored), ░ = used (dimmed)
pub fn render_usage_bar(label: &str, utilization: f64, resets_at: &str, indent: usize) {
    let remaining = 1.0 - utilization.clamp(0.0, 1.0);
    let pct_left = (remaining * 100.0).round() as u32;
    let bar_width = 20;
    let remaining_blocks = (remaining * bar_width as f64).round() as usize;
    let used_blocks = bar_width - remaining_blocks;

    let bar_color = if pct_left > 50 {
        "green"
    } else if pct_left > 20 {
        "yellow"
    } else {
        "red"
    };

    // Remaining shown as solid blocks (colored), used shown as outline blocks (dimmed)
    let remaining_str = "█".repeat(remaining_blocks);
    let used_str = "░".repeat(used_blocks);

    let bar = match bar_color {
        "green" => format!("{}{}", remaining_str.green(), used_str.dimmed()),
        "yellow" => format!("{}{}", remaining_str.yellow(), used_str.dimmed()),
        "red" => format!("{}{}", remaining_str.red(), used_str.dimmed()),
        _ => format!("{}{}", remaining_str, used_str),
    };

    let prefix = " ".repeat(indent);
    println!(
        "{}{}: {} {}% left {}",
        prefix,
        label,
        bar,
        pct_left,
        format!("(resets {})", resets_at).dimmed()
    );
}

/// Render a profile header badge like codex-profiles
pub fn render_profile_header(plan: &str, email: &str, label: &str, is_active: bool) {
    let plan_badge = format!(" {} ", plan.to_uppercase()).on_yellow().black().bold();
    let email_display = format!("  {}  ", email).on_bright_black().white();
    let label_display = if !label.is_empty() {
        format!("  {}  ", label).on_bright_black().white().to_string()
    } else {
        String::new()
    };

    let active_marker = if is_active {
        "  <- active".green().bold().to_string()
    } else {
        String::new()
    };

    println!();
    println!(
        "  {}{}{}{}",
        plan_badge, email_display, label_display, active_marker
    );
    println!();
}

/// Render a profile header with tool tag
pub fn render_profile_header_with_tool(
    tool: &str,
    plan: &str,
    email: &str,
    label: &str,
    is_active: bool,
) {
    // Claude = Anthropic brand terracotta #D97757, Codex = yellow
    let plan_badge = match tool {
        "claude" => format!(" {} ", plan.to_uppercase())
            .on_truecolor(217, 119, 87)
            .white()
            .bold(),
        _ => format!(" {} ", plan.to_uppercase())
            .on_yellow()
            .black()
            .bold(),
    };
    let email_display = format!("  {}  ", email).on_bright_black().white();
    let label_display = if !label.is_empty() {
        format!("  {}  ", label).on_bright_black().white().to_string()
    } else {
        String::new()
    };

    let active_marker = if is_active {
        let marker_text = match tool {
            "claude" => format!("  <- active ({})", tool)
                .truecolor(217, 119, 87)
                .bold()
                .to_string(),
            _ => format!("  <- active ({})", tool)
                .green()
                .bold()
                .to_string(),
        };
        marker_text
    } else {
        String::new()
    };

    println!();
    println!(
        "  {}{}{}{}",
        plan_badge, email_display, label_display, active_marker
    );
    println!();
}

// ─── Dashboard Row (compact single-line) rendering ───

/// Color tier for a weekly remaining percentage
#[derive(Clone, Copy)]
pub enum UsageTier {
    Green,  // >50% remaining
    Yellow, // 1-50% remaining
    Red,    // 0% remaining
}

impl UsageTier {
    pub fn from_remaining_pct(pct: u32) -> Self {
        if pct > 50 {
            UsageTier::Green
        } else if pct > 0 {
            UsageTier::Yellow
        } else {
            UsageTier::Red
        }
    }

    /// Status glyph for this tier
    pub fn glyph(&self) -> String {
        match self {
            UsageTier::Green => "\u{25CF}".truecolor(34, 197, 94).to_string(),  // ●
            UsageTier::Yellow => "\u{25D0}".truecolor(234, 179, 8).to_string(), // ◐
            UsageTier::Red => "\u{25CB}".truecolor(239, 68, 68).to_string(),    // ○
        }
    }

    /// Color a string according to this tier
    pub fn color_str(&self, s: &str) -> String {
        match self {
            UsageTier::Green => s.truecolor(34, 197, 94).to_string(),
            UsageTier::Yellow => s.truecolor(234, 179, 8).to_string(),
            UsageTier::Red => s.truecolor(239, 68, 68).to_string(),
        }
    }
}

/// Info needed to render a single dashboard row
pub struct DashboardRow {
    pub tool: String,
    pub plan: String,
    pub label: String,
    pub email: String,
    pub is_active: bool,
    /// Weekly remaining as 0-100 percentage. None if unknown.
    pub weekly_remaining_pct: Option<u32>,
    /// 5h remaining as 0-100 percentage. None if unknown.
    pub five_hour_remaining_pct: Option<u32>,
    /// Weekly reset time string (compact). Empty if unknown.
    pub weekly_reset: String,
    /// Extra credits suffix, e.g. "+$200". Empty if none.
    pub extra_credits: String,
    /// Cache suffix, e.g. "(cached 5m ago)". Empty if none.
    pub cache_suffix: String,
    /// Error message if fetch failed. Empty if success.
    pub error: String,
}

/// Build a colored bar of given width from a remaining percentage
fn build_bar(remaining_pct: u32, width: usize, tier: Option<UsageTier>) -> String {
    let filled = ((remaining_pct as f64 / 100.0) * width as f64).round() as usize;
    let empty = width - filled;
    let filled_str = "\u{2588}".repeat(filled); // █
    let empty_str = "\u{2591}".repeat(empty);   // ░

    if let Some(t) = tier {
        format!("{}{}", t.color_str(&filled_str), empty_str.dimmed())
    } else {
        // Neutral slate color for 5h bar
        format!(
            "{}{}",
            filled_str.truecolor(148, 163, 184),
            empty_str.dimmed()
        )
    }
}

/// Render a single dashboard row to stdout
pub fn render_dashboard_row(row: &DashboardRow) {
    let weekly_pct = row.weekly_remaining_pct.unwrap_or(0);
    let tier = UsageTier::from_remaining_pct(weekly_pct);
    let depleted = weekly_pct == 0 && row.error.is_empty();

    // 1. Status glyph
    let glyph = tier.glyph();

    // 2. Plan badge
    let plan_text = format!(" {} ", row.plan.to_uppercase());
    let plan_badge = match row.tool.as_str() {
        "claude" => plan_text.on_truecolor(217, 119, 87).white().bold().to_string(),
        _ => plan_text.on_yellow().black().bold().to_string(),
    };

    // 3. Label (use email username if no label), padded to 12 chars
    let display_name = if !row.label.is_empty() {
        row.label.clone()
    } else {
        row.email.split('@').next().unwrap_or(&row.email).to_string()
    };
    let name_padded = format!("{:<12}", display_name);

    // 4. Email (dimmed, truncated to 20 chars)
    let email_display = if row.email.len() > 20 {
        format!("{:.20}", row.email)
    } else {
        format!("{:<20}", row.email)
    };

    // 5. Weekly bar (12 chars wide) — hero metric
    let weekly_bar = if row.error.is_empty() {
        build_bar(weekly_pct, 12, Some(tier))
    } else {
        " ".repeat(12)
    };

    // 6. 5h bar (8 chars wide) — neutral slate
    let five_hour_pct = row.five_hour_remaining_pct.unwrap_or(100);
    let five_hour_bar = if row.error.is_empty() {
        build_bar(five_hour_pct, 8, None)
    } else {
        " ".repeat(8)
    };

    // 7. Reset time
    let reset_display = if !row.weekly_reset.is_empty() && row.error.is_empty() {
        format!("resets {}", row.weekly_reset).dimmed().to_string()
    } else {
        String::new()
    };

    // 8. Active marker
    let active_marker = if row.is_active {
        match row.tool.as_str() {
            "claude" => "<- active".truecolor(217, 119, 87).bold().to_string(),
            _ => "<- active".yellow().bold().to_string(),
        }
    } else {
        String::new()
    };

    // 9. Extra credits
    let credits_display = if !row.extra_credits.is_empty() {
        row.extra_credits.dimmed().to_string()
    } else {
        String::new()
    };

    // 10. Cache suffix
    let cache_display = if !row.cache_suffix.is_empty() {
        row.cache_suffix.dimmed().to_string()
    } else {
        String::new()
    };

    // Error line
    if !row.error.is_empty() {
        let line = format!(
            "  {}  {}  {}  {}  {}",
            glyph,
            plan_badge,
            name_padded,
            email_display.dimmed(),
            row.error.red(),
        );
        if depleted {
            println!("{}", line.dimmed());
        } else {
            println!("{}", line);
        }
        return;
    }

    // Build suffixes
    let mut suffixes: Vec<String> = Vec::new();
    if !reset_display.is_empty() {
        suffixes.push(reset_display);
    }
    if !active_marker.is_empty() {
        suffixes.push(active_marker);
    }
    if !credits_display.is_empty() {
        suffixes.push(credits_display);
    }
    if !cache_display.is_empty() {
        suffixes.push(cache_display);
    }
    let suffix_str = if suffixes.is_empty() {
        String::new()
    } else {
        format!("  {}", suffixes.join("  "))
    };

    let line = format!(
        "  {}  {}  {}  {} {} {}{}",
        glyph,
        plan_badge,
        name_padded,
        email_display.dimmed(),
        weekly_bar,
        five_hour_bar,
        suffix_str,
    );

    if depleted {
        // Print entire line dimmed — we rebuild without ANSI colors
        let line_plain = format!(
            "  {}  {}  {}  {} {} {}{}",
            "\u{25CB}",
            format!(" {} ", row.plan.to_uppercase()),
            name_padded,
            email_display,
            "\u{2591}".repeat(12),
            build_bar(five_hour_pct, 8, None),
            if suffixes.is_empty() { String::new() } else { format!("  {}", row.weekly_reset) },
        );
        println!("{}", line_plain.dimmed());
    } else {
        println!("{}", line);
    }
}
