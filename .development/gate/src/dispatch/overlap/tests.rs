#![cfg(test)]

use super::probes;

#[test]
fn a_literal_path_is_its_own_probe() {
    let paths = ["crates/betterlite-config/src/raw.rs".to_owned()];
    assert_eq!(probes(&paths), ["crates/betterlite-config/src/raw.rs"]);
}

#[test]
fn a_glob_probes_inside_its_folder() {
    let paths = ["crates/betterlite-http/src/proxy/**".to_owned()];
    assert_eq!(probes(&paths), ["crates/betterlite-http/src/proxy/probe"]);
}

#[test]
fn braces_expand_before_the_glob_cut() {
    let paths = ["crates/betterlite-http/src/{lib,proxy}.rs".to_owned()];
    assert_eq!(
        probes(&paths),
        [
            "crates/betterlite-http/src/lib.rs",
            "crates/betterlite-http/src/proxy.rs"
        ]
    );
}
