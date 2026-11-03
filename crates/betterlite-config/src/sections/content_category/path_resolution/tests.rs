#![cfg(test)]

#[test]
fn path_resolution_jails_and_resolves_relative_paths() {
    super::tests::path_resolution_uses_root_and_rejects_escape();
}
