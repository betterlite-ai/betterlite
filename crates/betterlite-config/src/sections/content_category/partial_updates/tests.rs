#![cfg(test)]

#[test]
fn partial_updates_preserve_prior_successful_loads() {
    super::tests::partial_updates_survive_later_load_failure();
}
