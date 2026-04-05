# aps

One CLI for all your coding agent accounts.

Switch between Claude Code and Codex profiles. See usage across all of them at once.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/andreasasprou/aps/main/install.sh | sh
```

Or with Rust:

```bash
cargo install --git https://github.com/andreasasprou/aps
```

## Why

You have multiple Anthropic accounts. Multiple OpenAI accounts. You're juggling rate limits across all of them. Nothing gives you a unified view.

`aps` does.

## Status

See every account's rate limits in one shot. Sorted by availability — the account you should use is always on top.

```
$ aps status --all

  ─── Claude Code
                                              weekly             5 hour
 ●   MAX   julia        pishitejulii@gmail.c  █████████░░░  76%  ████████  98%  resets 10 Apr
 ◐   MAX   intavia      andreas@intavia.ai    ███░░░░░░░░░  22%  ████████ 100%  resets 7 Apr
 ◐   MAX   andyasprou   andyasprou@gmail.com  █░░░░░░░░░░░  11%  ████████ 100%  resets 7 Apr  +$6
 ○   MAX   dweet        andreas@dweet.com     ░░░░░░░░░░░░   0%  ░░░░░░░░ 100%  resets 14:00

  ─── Codex
                                              weekly             5 hour
 ●   PRO   andyasprou   andyasprou@gmail.com  ██████████░░  81%  ████████  98%  resets 8 Apr
 ◐   PRO   dweet        andreas@dweet.com     █░░░░░░░░░░░   8%  ████████  97%  resets 8 Apr
 ○   PRO   julia        pishitejulii@gmail.c  ░░░░░░░░░░░░   0%  ░░░░░░░░ 100%  resets 8 Apr
```

**Status glyphs:**
- `●` green — good (>50% weekly remaining)
- `◐` yellow — low (1-50% weekly remaining)
- `○` dimmed — depleted (0% weekly, unusable until reset)

Rate-limited fetches fall back to cached data with a "(cached 5m ago)" indicator.

## Quick start

### Authenticate accounts (recommended)

Opens your browser for OAuth — gives full-scope tokens with 1-year expiry and auto-refresh:

```bash
# Claude accounts
aps auth claude --label dweet
aps auth claude --label work
aps auth claude --label personal

# Codex accounts
aps auth codex --label dweet
aps auth codex --label work
```

### Or save from existing credentials

```bash
# Save whatever's currently active in Claude Code / Codex
aps save claude
aps save codex

# Save from a setup token (claude setup-token output)
aps save claude --from-token <token> --label myaccount

# Save from a refresh token
aps save claude --from-refresh-token <token> --label myaccount
```

### Switch profiles

```bash
aps load claude    # Interactive picker — writes to keychain + credentials file
aps load codex     # Interactive picker — writes to auth.json
```

### See usage

```bash
aps status         # Active profiles only
aps status --all   # All profiles, sorted by availability
```

## Commands

```
aps auth claude [--label NAME]           Authenticate via OAuth (opens browser)
aps auth codex [--label NAME]            Authenticate via OAuth (opens browser)
aps save <claude|codex>                  Save current auth as a profile
aps save claude --from-token <TOKEN>     Save from a setup token
aps save claude --from-refresh-token <T> Save from a refresh token
aps load <claude|codex>                  Switch to a saved profile (interactive)
aps list [--tool claude|codex]           List all saved profiles
aps current [--tool claude|codex]        Show active profile per tool
aps status                               Usage for active profiles
aps status --all [--tool claude|codex]   Usage for ALL profiles
aps delete <claude|codex>                Delete profiles (interactive)
aps label set <tool> <id> <label>        Set a profile label
aps label clear <tool> <id>              Clear a profile label
aps label rename <tool> <from> <to>      Rename a label
aps costs                                Claude Code session stats
aps doctor                               Run diagnostics
```

## How it works

**Authentication:**
- `aps auth` runs a full OAuth PKCE flow — opens your browser, gets tokens with all scopes, saves the profile. Each auth creates an independent session that doesn't interfere with other machines.
- `aps save` captures whatever's currently active in Claude Code / Codex.
- `aps load` writes credentials to both the macOS Keychain and `~/.claude/.credentials.json` (Claude) or `~/.codex/auth.json` (Codex).

**Usage fetching:**
- Claude: `api.anthropic.com/api/oauth/usage` with token refresh on 401/403
- Codex: `chatgpt.com/backend-api/wham/usage` with token refresh on 401/403
- Refreshed tokens are persisted back to profile files
- Rate-limited responses fall back to cached data
- Claude calls are sequential (3s gaps) to avoid rate limits; Codex calls are parallel

**Storage:** Profiles in `~/.aps/`. Atomic writes. File locking. No database.

## Built with

Rust. [clap](https://github.com/clap-rs/clap) for CLI. [inquire](https://github.com/mikaelmello/inquire) for interactive prompts. [comfy-table](https://github.com/Nukesor/comfy-table) for aligned output. [colored](https://github.com/colored-rs/colored) for terminal styling. [tiny_http](https://github.com/tiny-http/tiny-http) for OAuth callbacks.
