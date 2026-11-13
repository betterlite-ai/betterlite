use std::path::PathBuf;

use crate::loader::EnvironmentLookup;

pub(super) fn expand<L>(lookup: &L, text: &str) -> PathBuf
where
    L: EnvironmentLookup,
{
    let expanded =
        shellexpand::full_with_context_no_errors(text, || home(lookup), |name| lookup.lookup(name));
    PathBuf::from(expanded.into_owned())
}

fn home<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    if let Some(home) = lookup.lookup("HOME").filter(|home| !home.is_empty()) {
        return Some(home);
    }
    let path = lookup.system_home()?;
    path.to_str().map(str::to_owned)
}
