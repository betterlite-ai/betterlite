pub mod command;
pub mod error;
pub mod login;
pub mod ports;
mod restore;
mod runtime;
#[cfg(unix)]
pub mod signals;
mod tests;

pub const UP_ABOUT: &str =
    "Route every Claude Code session through your LiteLLM proxy until stopped";

pub const UP_LONG_ABOUT: &str = "Route every Claude Code session through your LiteLLM proxy until stopped.\n\nPatches ~/.claude/settings.json so Claude Code picks up the proxy on its own next startup, from any terminal -- no need to launch it through `lite`. The key written is the one this command resolved (your fresh `lite login`, or an explicit --api-key), copied in as a static token for as long as `up` runs. Press Ctrl-C to stop and restore your original settings. Assumes the proxy is already running (this does not start one for you). Cursor is not supported: it has no equivalent file-based config to patch.";
