#![cfg(test)]

use crate::show::components;

#[test]
fn components_are_the_crates_in_scope() {
    let paths = [
        "crates/betterlite-server/src/**".to_owned(),
        "crates/betterlite-server/Cargo.toml".to_owned(),
        "crates/betterlite-config/src/env.rs".to_owned(),
        "tools/oracle/cases/models/**".to_owned(),
        "Cargo.toml".to_owned(),
    ];
    assert_eq!(
        components(&paths),
        ["betterlite-config", "betterlite-server"]
    );
}
