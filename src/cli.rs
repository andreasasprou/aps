use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "aps",
    about = "Agent Profile Switcher — manage Claude Code and Codex auth profiles",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Disable colors and styling
    #[arg(long, global = true)]
    pub plain: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Save current auth as a named profile
    Save {
        /// Tool to save profile for (claude or codex)
        tool: String,

        /// Save from a setup token (1-year access token from `claude setup-token`).
        /// Also accepts CLAUDE_CODE_OAUTH_TOKEN values.
        #[arg(long)]
        from_token: Option<String>,

        /// Save from a refresh token (from ~/.claude/.credentials.json).
        #[arg(long, conflicts_with = "from_token")]
        from_refresh_token: Option<String>,

        /// Label for the profile (skips interactive prompt)
        #[arg(long, short)]
        label: Option<String>,
    },
    /// Load a saved profile (interactive picker)
    Load {
        /// Tool to load profile for (claude or codex)
        tool: String,
    },
    /// List all saved profiles
    List {
        /// Filter by tool (claude or codex)
        tool: Option<String>,
    },
    /// Show active profile for each tool
    Current {
        /// Filter by tool (claude or codex)
        tool: Option<String>,
    },
    /// Show usage stats with progress bars
    Status {
        /// Show usage for all profiles, not just active
        #[arg(long)]
        all: bool,
        /// Filter by tool (claude or codex)
        #[arg(long)]
        tool: Option<String>,
    },
    /// Delete a saved profile
    Delete {
        /// Tool (claude or codex)
        tool: String,
    },
    /// Manage profile labels
    Label {
        #[command(subcommand)]
        command: LabelCommands,
    },
    /// Show Claude Code usage stats (sessions, tokens, daily activity)
    Costs,
    /// Run diagnostics and check configuration
    Doctor,
    /// Authenticate with a tool via OAuth
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
}

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Authenticate with Claude via OAuth PKCE (opens browser)
    Claude {
        /// Optional label for the saved profile
        #[arg(long, short)]
        label: Option<String>,
        /// Paste authorization code manually (for headless/SSH machines)
        #[arg(long)]
        manual: bool,
    },
    /// Authenticate with Codex via OAuth PKCE
    Codex {
        /// Optional label for the saved profile
        #[arg(long, short)]
        label: Option<String>,
        /// Use device-code auth for headless/SSH machines
        #[arg(long)]
        device_auth: bool,
    },
}

#[derive(Subcommand)]
pub enum LabelCommands {
    /// Set a label on a profile
    Set {
        /// Tool (claude or codex)
        tool: String,
        /// Profile ID
        id: String,
        /// Label to set
        label: String,
    },
    /// Clear a label from a profile
    Clear {
        /// Tool (claude or codex)
        tool: String,
        /// Profile ID
        id: String,
    },
    /// Rename a label
    Rename {
        /// Tool (claude or codex)
        tool: String,
        /// Current label
        from: String,
        /// New label
        to: String,
    },
}
