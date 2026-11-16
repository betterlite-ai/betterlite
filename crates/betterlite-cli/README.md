# betterlite Proxy CLI

`lite` is the command-line tool for a betterlite proxy server. It manages models, credentials, keys, users, teams, the stored configuration and coding-agent routing, and it can start an interactive shell.

## Installation

Build the binary from the workspace, or install it with Cargo, which installs it as `lite`:

```bash
cargo build -p betterlite-cli
cargo install --path crates/betterlite-cli
```

## Configuration

The CLI reads its connection settings from environment variables, command-line options, or a persistent config file:

- `LITELLM_PROXY_URL`: base URL of the proxy server (default: `http://localhost:4000`)
- `LITELLM_PROXY_API_KEY`: API key for authentication

Store the URL once instead of exporting it in every shell:

```bash
lite config set base_url https://your-proxy.example.com
lite config get base_url
lite config unset base_url
```

The base URL is resolved in this order of precedence:

1. `--base-url`
2. `LITELLM_PROXY_URL`
3. `base_url` from `~/.litellm/config.json`
4. `http://localhost:4000`

A stored API key in the token file is used only when neither `--api-key` nor `LITELLM_PROXY_API_KEY` gives one, and never by `configure`, `reconfigure` or `unconfigure`.

### Hiding commands from the listings

Deployments that hand `lite` to end users often want to advertise only part of it. Store the commands to keep out of the listings, comma separated:

```bash
lite config set hidden_commands codex,opencode
lite config unset hidden_commands
```

Hidden commands drop out of both `lite --help` and the interactive shell's "Available commands" block, and stay runnable so existing scripts keep working.

## Global Options

- `--base-url`: base URL of the proxy server
- `--api-key`: API key for authentication
- `--version`, `-v`: print the CLI and server version and exit

```bash
lite version
lite --version
lite -v
```

## Commands

Run `lite --help` for the full list. Running `lite` with no command starts the interactive shell, where `help`, `clear`, `exit` and `quit` are handled locally and every other line runs as a normal command.

## Auto-router aliases

`lite autoroute up` and `lite autoroute down` still work as hidden aliases of `lite autoroute start` and `lite autoroute stop`; use the new names in scripts.
