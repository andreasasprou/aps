# aps

One CLI for all your coding agent accounts.

Switch between Claude Code and Codex profiles. See usage across all of them at once.

```
$ cargo install --path .
```

## Why

You have multiple Anthropic accounts. Multiple OpenAI accounts. You're juggling rate limits across all of them. `codex-profiles` handles Codex. Nothing handles Claude. Nothing handles both.

`aps` does.

## Status

See every account's rate limits in one shot. All fetched in parallel.

```
$ aps status --all

 MAX   andyasprou@gmail.com    personal    <- active (claude)

    5 hour: ░░░░░░░░░░░░░░░░░░░░ 0% left (resets 19:00)
    Weekly: ░░░░░░░░░░░░░░░░░░░░ 0% left (resets 20:00)
    sonnet:
      Weekly: ░░░░░░░░░░░░░░░░░░░░ 0% left (resets 20:00)
    Extra credits: 0.00/10000.00 USD used

 PRO   andreas@dweet.com    dweet

    codex:
      5 hour: ████████████████████ 99% left (resets 17:21)
      Weekly: ██░░░░░░░░░░░░░░░░░░ 10% left (resets 10:04 on 3 Apr)

 PRO   pishitejulii@gmail.com    julia    <- active (codex)

    codex:
      5 hour: ███░░░░░░░░░░░░░░░░░ 15% left (resets 17:07)
      Weekly: ██████████████░░░░░░ 71% left (resets 19:36 on 4 Apr)
```

## Commands

```
aps save <claude|codex>      Save current auth as a profile
aps load <claude|codex>      Switch to a saved profile (interactive)
aps list                     List all saved profiles
aps current                  Show active profile per tool
aps status                   Usage for active profiles
aps status --all             Usage for ALL profiles
aps delete <claude|codex>    Delete profiles (interactive)
aps label set <tool> <id> <label>
aps label clear <tool> <id>
aps label rename <tool> <from> <to>
aps doctor                   Diagnostics
```

## How it works

**Claude Code:** Reads OAuth tokens from macOS Keychain. Fetches usage from `api.anthropic.com`. Account info from the OAuth account endpoint.

**Codex:** Reads `~/.codex/auth.json`. Decodes the JWT for email and plan. Fetches usage from `chatgpt.com/backend-api`.

Profiles stored in `~/.aps/`. Atomic writes. File locking. No database.

## Install

```
cargo install --path .
```

Or build from source:

```
cargo build --release
cp target/release/aps /usr/local/bin/
```

## Quick start

```
# Save your current accounts
aps save claude
aps save codex

# See everything
aps status --all

# Switch accounts
aps load codex
aps load claude
```

## Built with

Rust. [clap](https://github.com/clap-rs/clap) for CLI. [inquire](https://github.com/mikaelmello/inquire) for interactive prompts. [colored](https://github.com/colored-rs/colored) for terminal styling. Parallel HTTP via threads + channels.
