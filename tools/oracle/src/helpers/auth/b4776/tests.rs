#![cfg(test)]

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::helpers::Helper as _;

use super::Op;

const ALPHA_STABLE: &str = "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8";
const ALPHA_LEGACY: &str = "00afdb1e315efd7d8d96382f3925c7d46dee40711cffe80449b0dcb21f731322";
const BETA_STABLE: &str = "635891353974cd5ae4427fe3ea7ed477e8b74f703ad175fa74d29a7c96e869f5";
const BETA_LEGACY: &str = "ab79e4913c6aaed7238fce84ab6b3c6db0f27cab4e6e0549dd2061e7e8c5ec17";
const FLOATS_STABLE: &str = "4b379441613442e9bd746f463554d9e27aa22d4d4aa4aa7c84f3192997bc2200";
const FLOATS_LEGACY: &str = "da711e04632a269baea7837b076f61e3309bd15ae9c5353ce63650ee4434e55a";
const TWIN_STABLE: &str = "72b33a1cb0bfc9cdd3db0102962414c7a0d85aad94eba64cd8c33265242f7f9f";
const TWIN_FIRST_LEGACY: &str = "3c7e3968e1ad8328444e789f484a796aa09ff66ccea3ce23593a14c882f9cd79";
const TWIN_SECOND_LEGACY: &str = "3135a366984edaf87e84ab20f6c2d788be57f8c9d00012eaab77122f8282abf4";
const KEPT_STABLE: &str = "79f076abdd19a752db7267bfff2f9022161d120dea919fdaca2ffdfc24ca8c96";
const KEPT_LEGACY: &str = "0db21a4e8ded5f3d6415e017a2169f868611aa3eb62c60149a8e5da98fb705c7";
const DBBY_STABLE: &str = "227bec3c763b4fc5817163df108f2ecf2bedc2168e3a355491cbc1f651269d73";
const DBBY_LEGACY: &str = "3bd9ad8ec6ba7e729ee6a7740ef9d52b8dd9417d5ec1d7913cb58b5c5626e32e";
const BETA_CONFIG_STABLE: &str = "f44e64e75f3948e9f73f8dfa94721c4ce8cbb4f265c4790c702b2d41cfbf2753";
const BETA_CONFIG_LEGACY: &str = "4fb4045f0abf0ca2d4659df3e9686c86d358c77c2ed700195e5b3f52e4d406b1";

fn run(fixture: &str) -> Value {
    let case: HelperCase = serde_saphyr::from_str(fixture).unwrap();
    Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn an_ascii_entry_maps_its_legacy_id_to_the_stable_id() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/ids-ascii-entry.yaml"
    ));
    assert_eq!(
        output["ids_for"][ALPHA_STABLE],
        json!([ALPHA_LEGACY, ALPHA_STABLE])
    );
    assert_eq!(output["ids_for"][ALPHA_LEGACY], json!([ALPHA_LEGACY]));
    assert_eq!(
        output["stable_ids"],
        json!([ALPHA_STABLE, ALPHA_STABLE, "missing"])
    );
}

#[test]
fn an_entry_with_escapes_and_non_ascii_hides_the_legacy_id_from_a_legacy_lookup() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/ids-non-ascii-and-escapes.yaml"
    ));
    assert_eq!(
        output["ids_for"][BETA_STABLE],
        json!([BETA_STABLE, BETA_LEGACY])
    );
    assert_eq!(output["stable_ids"], json!([BETA_STABLE]));
    assert_eq!(output["agents"][0]["agent_name"], json!("béta"));
}

#[test]
fn floats_and_nested_values_hash_like_python_json_dumps() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/ids-floats-and-nesting.yaml"
    ));
    assert_eq!(
        output["ids_for"][FLOATS_STABLE],
        json!([FLOATS_STABLE, FLOATS_LEGACY])
    );
    assert_eq!(output["stable_ids"], json!([FLOATS_STABLE]));
}

#[test]
fn entries_without_a_name_or_card_are_skipped_and_never_enter_the_legacy_map() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/config-load-skips-incomplete-entries.yaml"
    ));
    assert_eq!(output["agents"].as_array().map(Vec::len), Some(1));
    assert_eq!(output["agents"][0]["agent_id"], json!(KEPT_STABLE));
    assert_eq!(output["by_name"]["kept"]["agent_id"], json!(KEPT_STABLE));
    assert_eq!(
        output["ids_for"][KEPT_STABLE],
        json!([KEPT_LEGACY, KEPT_STABLE])
    );
}

#[test]
fn a_repeated_config_name_registers_once_but_keeps_every_legacy_alias() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/config-load-name-collision.yaml"
    ));
    assert_eq!(output["agents"].as_array().map(Vec::len), Some(1));
    assert_eq!(output["agents"][0]["agent_id"], json!(TWIN_STABLE));
    assert_eq!(
        output["ids_for"][TWIN_STABLE],
        json!([TWIN_SECOND_LEGACY, TWIN_FIRST_LEGACY, TWIN_STABLE])
    );
    assert_eq!(output["by_name"]["twin"]["agent_id"], json!(TWIN_STABLE));
}

#[test]
fn a_config_entry_skipped_for_a_database_name_still_maps_its_legacy_id() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/config-load-legacy-map-includes-collision.yaml"
    ));
    assert_eq!(output["agents"].as_array().map(Vec::len), Some(1));
    assert_eq!(output["agents"][0]["agent_id"], json!("db-dbby"));
    assert_eq!(
        output["ids_for"][DBBY_STABLE],
        json!([DBBY_STABLE, DBBY_LEGACY])
    );
    assert_eq!(output["ids_for"]["db-dbby"], json!(["db-dbby"]));
    assert_eq!(output["stable_ids"], json!([DBBY_STABLE]));
}

#[test]
fn a_database_agent_wins_over_a_config_entry_with_its_name() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/reload-db-wins-over-config.yaml"
    ));
    assert_eq!(
        output["agents"],
        json!([
            {"agent_id": "db-alpha", "agent_name": "alpha"},
            {"agent_id": BETA_CONFIG_STABLE, "agent_name": "beta"},
        ])
    );
    assert_eq!(output["by_id"]["db-alpha"]["agent_name"], json!("alpha"));
    assert_eq!(output["by_name"]["alpha"]["agent_id"], json!("db-alpha"));
    assert_eq!(
        output["by_name"]["beta"]["agent_id"],
        json!(BETA_CONFIG_STABLE)
    );
}

#[test]
fn a_reload_without_a_config_keeps_the_agents_the_last_load_remembered() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/reload-remembers-config.yaml"
    ));
    assert_eq!(
        output["agents"],
        json!([
            {"agent_id": "db-gamma", "agent_name": "gamma"},
            {"agent_id": ALPHA_STABLE, "agent_name": "alpha"},
            {"agent_id": BETA_CONFIG_STABLE, "agent_name": "beta"},
        ])
    );
}

#[test]
fn an_exact_id_match_wins_over_a_legacy_alias() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/lookup-exact-then-legacy.yaml"
    ));
    assert_eq!(output["by_id"][ALPHA_LEGACY]["agent_name"], json!("shadow"));
    assert_eq!(output["by_id"][ALPHA_STABLE]["agent_name"], json!("alpha"));
    assert_eq!(output["by_id"]["missing"], Value::Null);
    assert_eq!(output["by_name"]["alpha"]["agent_id"], json!(ALPHA_STABLE));
    assert_eq!(output["by_name"]["missing"], Value::Null);
}

#[test]
fn a_legacy_id_resolves_to_its_stable_id_only_in_that_direction() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/ids-for-agent-and-stable-id.yaml"
    ));
    assert_eq!(
        output["ids_for"][ALPHA_STABLE],
        json!([ALPHA_LEGACY, ALPHA_STABLE])
    );
    assert_eq!(output["ids_for"][ALPHA_LEGACY], json!([ALPHA_LEGACY]));
    assert_eq!(
        output["ids_for"][BETA_CONFIG_STABLE],
        json!([BETA_CONFIG_LEGACY, BETA_CONFIG_STABLE])
    );
    assert_eq!(
        output["stable_ids"],
        json!([
            ALPHA_STABLE,
            BETA_CONFIG_STABLE,
            BETA_CONFIG_STABLE,
            "missing",
        ])
    );
}

#[test]
fn a_public_group_matching_a_legacy_alias_lists_the_agent() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/public-list-legacy-alias.yaml"
    ));
    assert_eq!(output["agents"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        output["public"],
        json!([{"agent_id": ALPHA_STABLE, "agent_name": "alpha"}])
    );
}

#[test]
fn deregistration_removes_every_agent_under_a_name() {
    let output = run(include_str!(
        "../../../../cases/agent-registry/deregister-removes-all-by-name.yaml"
    ));
    assert_eq!(output["agents"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        output["after_deregister"],
        json!([{"agent_id": "b1", "agent_name": "beta"}])
    );
}
