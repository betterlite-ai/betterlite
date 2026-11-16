# betterlite Proxy CLI

`lite` manages a betterlite proxy server: models, credentials, keys, users, teams, the stored configuration and coding-agent routing, plus an interactive shell.

## Installation

```bash
cargo build -p betterlite-cli
cargo install --path crates/betterlite-cli
```

Cargo installs the binary as `lite`.

## Configuration

`LITELLM_PROXY_URL` sets the proxy URL (default `http://localhost:4000`) and `LITELLM_PROXY_API_KEY` the API key. Settings can be stored once instead of exported in every shell:

```bash
lite config set base_url https://your-proxy.example.com
lite config get base_url
lite config unset base_url
```

The base URL comes from `--base-url`, then `LITELLM_PROXY_URL`, then `base_url` in `~/.litellm/config.json`, then the default. A token-file key is used only when neither `--api-key` nor `LITELLM_PROXY_API_KEY` gives one, and never by `configure`, `reconfigure` or `unconfigure`.

Deployments that advertise only part of the CLI store a comma-separated `hidden_commands`. A hidden command drops out of `lite --help` and the shell's "Available commands" block but stays runnable:

```bash
lite config set hidden_commands codex,opencode
lite config unset hidden_commands
```

## Global Options

`--base-url` and `--api-key` come before the command; `--version` or `-v` prints the CLI and server version and exits. Running `lite` with no command starts the shell, where `help`, `clear`, `exit` and `quit` are handled locally and every other line runs as a normal command.

## Auto-router aliases

`lite autoroute up` and `lite autoroute down` are hidden aliases of `lite autoroute start` and `lite autoroute stop`; use the new names in scripts.
