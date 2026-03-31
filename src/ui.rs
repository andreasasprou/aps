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
