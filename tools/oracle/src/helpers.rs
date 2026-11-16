mod oracle;
mod router;

use std::sync::LazyLock;

use betterlite_oracle_helper_api::group::Group;

// the oracle's own two groups need its gateway modes, so they stay here; a static that derefs to a slice keeps `run_case(&GROUPS, ..)` call sites unchanged
pub(crate) static GROUPS: LazyLock<Vec<Group>> = LazyLock::new(|| {
    betterlite_oracle_groups::GROUPS
        .iter()
        .copied()
        .chain([oracle::GROUP, router::GROUP])
        .collect()
});

mod tests;
