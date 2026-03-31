use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Supported tools
pub const TOOLS: &[&str] = &["claude", "codex"];

pub fn validate_tool(tool: &str) -> Result<&str> {
    match tool {
        "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        _ => anyhow::bail!("Unknown tool '{}'. Supported: claude, codex", tool),
    }
}

pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

/// ~/.aps/ — our profile storage root
pub fn aps_dir() -> Result<PathBuf> {
    let dir = home_dir()?.join(".aps");
    fs::create_dir_all(&dir).context("Failed to create ~/.aps directory")?;
    Ok(dir)
}

/// ~/.aps/claude/profiles/ or ~/.aps/codex/profiles/
pub fn profiles_dir(tool: &str) -> Result<PathBuf> {
    let dir = aps_dir()?.join(tool).join("profiles");
    fs::create_dir_all(&dir).context(format!("Failed to create profiles dir for {}", tool))?;
    Ok(dir)
}

/// ~/.aps/claude/profiles.json or ~/.aps/codex/profiles.json
pub fn profiles_index_path(tool: &str) -> Result<PathBuf> {
    Ok(aps_dir()?.join(tool).join("profiles.json"))
}

/// ~/.aps/claude/profiles.lock or ~/.aps/codex/profiles.lock
pub fn profiles_lock_path(tool: &str) -> Result<PathBuf> {
    let dir = aps_dir()?.join(tool);
    fs::create_dir_all(&dir)?;
    Ok(dir.join("profiles.lock"))
}

// Tool-specific auth file paths

/// ~/.claude/.credentials.json
pub fn claude_credentials_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude").join(".credentials.json"))
}

/// ~/.codex/auth.json
pub fn codex_auth_path() -> Result<PathBuf> {
    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().unwrap().join(".codex"));
    Ok(codex_home.join("auth.json"))
}

/// Atomically write a file (write to temp, then rename)
pub fn atomic_write(path: &PathBuf, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("No parent directory")?;
    fs::create_dir_all(parent)?;
    let temp_path = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temp_path, contents).context("Failed to write temp file")?;
    fs::rename(&temp_path, path).context("Failed to rename temp file")?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }

    Ok(())
}

/// Use directories crate as fallback for home dir
fn dirs_home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

mod dirs {
    use std::path::PathBuf;
    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(|| super::dirs_home())
    }
}
