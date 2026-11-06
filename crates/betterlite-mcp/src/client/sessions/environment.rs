use std::collections::BTreeMap;

// LiteLLM passes these variables through to an MCP stdio server and nothing else, so credentials never reach the child
pub const SAFE_STDIO_KEYS: [&str; 19] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SHELL",
    "LANG",
    "LC_ALL",
    "NPM_CONFIG_CACHE",
    "PNPM_HOME",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "SYSTEMROOT",
    "COMSPEC",
    "PATHEXT",
    "WINDIR",
];

#[must_use]
pub fn safe_stdio_env(
    provided: Option<&BTreeMap<String, String>>,
    environment: &BTreeMap<String, String>,
    npm_cache_dir: &str,
) -> BTreeMap<String, String> {
    if let Some(values) = provided {
        return values.clone();
    }
    let mut safe = BTreeMap::new();
    for key in SAFE_STDIO_KEYS {
        if let Some(value) = environment.get(key) {
            safe.insert(key.to_owned(), value.clone());
        }
    }
    safe.entry("NPM_CONFIG_CACHE".to_owned())
        .or_insert_with(|| npm_cache_dir.to_owned());
    safe
}
