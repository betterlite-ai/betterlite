#![cfg(test)]

use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::{
    AutoRouterCatalog, AutoRouterCatalogEntry, CatalogDeployment, CatalogLitellmParams,
    CatalogModelInfo, build_auto_router_catalog,
};

const TEST_SIGNING_KEY: &str = "auto-router-catalog-test-signing-key";
const OTHER_SIGNING_KEY: &str = "auto-router-catalog-other-signing-key";
const COMPLEXITY_MODEL: &str = "auto_router/complexity_router";

fn cipher_with(signing_key: Option<&str>) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: signing_key.map(SecretString::from),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn cipher() -> DbCipher {
    cipher_with(Some(TEST_SIGNING_KEY))
}

fn sealed(cipher: &DbCipher, text: &str) -> String {
    let sealed = cipher
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from(text.to_owned())),
            None,
        )
        .unwrap();
    let StoredValue::Text(value) = sealed else {
        panic!("a sealed text stays text");
    };
    value.expose_secret().to_owned()
}

fn row(text: &str) -> ProxyModelTable {
    serde_json::from_str(text).unwrap()
}

#[test]
fn catalog_keeps_only_complexity_routers_in_row_order() {
    let rows = vec![
        row(
            r#"{"model_id":"semantic","model_name":"semantic","litellm_params":{"model":"auto_router/semantic_router"}}"#,
        ),
        row(
            r#"{"model_id":"complexity","model_name":"complexity","litellm_params":{"model":"auto_router/complexity_router"}}"#,
        ),
        row(
            r#"{"model_id":"adaptive","model_name":"adaptive","litellm_params":{"model":"auto_router/adaptive_router"}}"#,
        ),
        row(
            r#"{"model_id":"quality","model_name":"quality","litellm_params":{"model":"auto_router/quality_router"}}"#,
        ),
        row(r#"{"model_id":"plain","model_name":"plain","litellm_params":{"model":"gpt-4o"}}"#),
        row(
            r#"{"model_id":"complexity-v2","model_name":"complexity-v2","litellm_params":{"model":"auto_router/complexity_router_v2"}}"#,
        ),
    ];
    let catalog = build_auto_router_catalog(&rows, &cipher()).unwrap();
    let model_ids = catalog
        .iter()
        .map(|entry| entry.model_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(model_ids, vec!["complexity", "complexity-v2"]);
}

#[test]
fn catalog_carries_team_and_creator_and_defaults_them() {
    let rows = vec![
        row(
            r#"{"model_id":"with-team","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":"team-a"},"created_by":"creator-a"}"#,
        ),
        row(
            r#"{"model_id":"without-info","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"created_by":"creator-b"}"#,
        ),
        row(
            r#"{"model_id":"empty-info","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{}}"#,
        ),
        row(
            r#"{"model_id":"null-info","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":null}"#,
        ),
        row(
            r#"{"model_id":"null-team","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":null}}"#,
        ),
    ];
    let catalog = build_auto_router_catalog(&rows, &cipher()).unwrap();
    assert_eq!(catalog[0].team_id.as_deref(), Some("team-a"));
    assert_eq!(catalog[0].created_by.as_deref(), Some("creator-a"));
    assert_eq!(catalog[1].team_id, None);
    assert_eq!(catalog[1].created_by.as_deref(), Some("creator-b"));
    assert_eq!(catalog[2].team_id, None);
    assert_eq!(catalog[2].created_by, None);
    assert_eq!(catalog[3].team_id, None);
    assert_eq!(catalog[4].team_id, None);
}

#[rstest]
#[case::missing(r#"{"model_id":"missing","model_name":"n","litellm_params":{}}"#)]
#[case::null(r#"{"model_id":"null","model_name":"n","litellm_params":{"model":null}}"#)]
#[case::number(r#"{"model_id":"number","model_name":"n","litellm_params":{"model":7}}"#)]
#[case::list(r#"{"model_id":"list","model_name":"n","litellm_params":{"model":["auto_router/complexity_router"]}}"#)]
#[case::object(r#"{"model_id":"object","model_name":"n","litellm_params":{"model":{"name":"auto_router/complexity_router"}}}"#)]
fn catalog_skips_a_row_whose_model_is_not_a_string(#[case] row_json: &str) {
    let catalog = build_auto_router_catalog(&[row(row_json)], &cipher()).unwrap();
    assert!(catalog.is_empty());
}

#[test]
fn catalog_decrypts_an_encrypted_model_and_config() {
    let sealed_model = sealed(&cipher(), COMPLEXITY_MODEL);
    let sealed_config = sealed(&cipher(), r#"{"classifier_type":"heuristic_v2"}"#);
    let row_json = json!({
        "model_id": "encrypted",
        "model_name": "encrypted",
        "litellm_params": {"model": sealed_model, "complexity_router_config": sealed_config},
    });
    let rows = vec![serde_json::from_value::<ProxyModelTable>(row_json).unwrap()];
    let catalog = build_auto_router_catalog(&rows, &cipher()).unwrap();
    assert_eq!(catalog[0].deployment.litellm_params.model, COMPLEXITY_MODEL);
    assert_eq!(
        catalog[0]
            .deployment
            .litellm_params
            .complexity_router_config,
        Some(JsonData::String(
            r#"{"classifier_type":"heuristic_v2"}"#.to_owned()
        ))
    );
}

#[test]
fn catalog_keeps_a_value_it_cannot_decrypt() {
    let sealed_config = sealed(&cipher_with(Some(OTHER_SIGNING_KEY)), "sealed-config-text");
    let row_json = json!({
        "model_id": "kept",
        "model_name": "kept",
        "litellm_params": {"model": COMPLEXITY_MODEL, "complexity_router_config": sealed_config.clone()},
    });
    let rows = vec![serde_json::from_value::<ProxyModelTable>(row_json).unwrap()];
    let catalog = build_auto_router_catalog(&rows, &cipher()).unwrap();
    assert_eq!(catalog[0].deployment.litellm_params.model, COMPLEXITY_MODEL);
    assert_eq!(
        catalog[0]
            .deployment
            .litellm_params
            .complexity_router_config,
        Some(JsonData::String(sealed_config))
    );
}

#[test]
fn catalog_keeps_stored_strings_when_no_signing_key_is_configured() {
    let sealed_model = sealed(&cipher(), COMPLEXITY_MODEL);
    let sealed_config = sealed(&cipher(), "sealed-config-text");
    let row_json = json!({
        "model_id": "keyless",
        "model_name": "keyless",
        "litellm_params": {"model": sealed_model, "complexity_router_config": sealed_config},
    });
    let rows = vec![serde_json::from_value::<ProxyModelTable>(row_json).unwrap()];
    let catalog = build_auto_router_catalog(&rows, &cipher_with(None)).unwrap();
    assert!(catalog.is_empty());
    let plain = row(
        r#"{"model_id":"plain","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":"stored-text"}}"#,
    );
    let plain_catalog = build_auto_router_catalog(&[plain], &cipher_with(None)).unwrap();
    assert_eq!(
        plain_catalog[0]
            .deployment
            .litellm_params
            .complexity_router_config,
        Some(JsonData::String("stored-text".to_owned()))
    );
    assert_eq!(
        plain_catalog[0].deployment.litellm_params.model,
        COMPLEXITY_MODEL
    );
}

#[rstest]
#[case::object(
    r#"{"model_id":"object","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":{"classifier_type":"heuristic_v2"}}}"#,
    r#"{"classifier_type":"heuristic_v2"}"#
)]
#[case::list(
    r#"{"model_id":"list","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":["tier"]}}"#,
    r#"["tier"]"#
)]
#[case::number(
    r#"{"model_id":"number","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":7}}"#,
    "7"
)]
#[case::flag(
    r#"{"model_id":"flag","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":false}}"#,
    "false"
)]
#[case::text(
    r#"{"model_id":"text","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":"plain-text"}}"#,
    r#""plain-text""#
)]
fn catalog_keeps_a_non_string_config_as_stored(#[case] row_json: &str, #[case] config_json: &str) {
    let catalog = build_auto_router_catalog(&[row(row_json)], &cipher()).unwrap();
    let stored: JsonData = serde_json::from_str(config_json).unwrap();
    assert_eq!(
        catalog[0]
            .deployment
            .litellm_params
            .complexity_router_config,
        Some(stored)
    );
}

#[rstest]
#[case::missing(r#"{"model_id":"missing","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"}}"#)]
#[case::null(r#"{"model_id":"null","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":null}}"#)]
fn catalog_nulls_a_missing_or_null_config(#[case] row_json: &str) {
    let catalog = build_auto_router_catalog(&[row(row_json)], &cipher()).unwrap();
    assert_eq!(
        catalog[0]
            .deployment
            .litellm_params
            .complexity_router_config,
        None
    );
}

#[rstest]
#[case::number(r#"{"model_id":"bad","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":7}}"#)]
#[case::flag(r#"{"model_id":"bad","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":true}}"#)]
#[case::list(r#"{"model_id":"bad","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":["team-a"]}}"#)]
#[case::object(r#"{"model_id":"bad","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":{"id":"team-a"}}}"#)]
#[case::skipped_row(r#"{"model_id":"bad","model_name":"n","litellm_params":{"model":"gpt-4o"},"model_info":{"team_id":7}}"#)]
fn catalog_drops_the_whole_catalog_for_a_non_string_team_id(#[case] bad_row: &str) {
    let rows = vec![
        row(
            r#"{"model_id":"good","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"},"model_info":{"team_id":"team-a"}}"#,
        ),
        row(bad_row),
    ];
    assert_eq!(build_auto_router_catalog(&rows, &cipher()), None);
}

#[test]
fn catalog_with_no_rows_is_empty() {
    let catalog = build_auto_router_catalog(&[], &cipher()).unwrap();
    assert!(catalog.is_empty());
}

#[test]
fn catalog_deployment_serializes_to_the_litellm_shape() {
    let rows = vec![row(
        r#"{"model_id":"m","model_name":"n","litellm_params":{"model":"auto_router/complexity_router","complexity_router_config":{"classifier_type":"heuristic_v2"}},"model_info":{"team_id":"team-a"},"created_by":"creator"}"#,
    )];
    let catalog = build_auto_router_catalog(&rows, &cipher()).unwrap();
    assert_eq!(
        serde_json::to_value(&catalog).unwrap(),
        json!([{
            "model_id": "m",
            "team_id": "team-a",
            "created_by": "creator",
            "deployment": {
                "litellm_params": {
                    "model": "auto_router/complexity_router",
                    "complexity_router_config": {"classifier_type": "heuristic_v2"},
                },
                "model_info": {"id": "m", "db_model": true},
            },
        }])
    );
}

#[test]
fn catalog_deployment_carries_a_null_config() {
    let rows = vec![row(
        r#"{"model_id":"no-config","model_name":"n","litellm_params":{"model":"auto_router/complexity_router"}}"#,
    )];
    let catalog = build_auto_router_catalog(&rows, &cipher()).unwrap();
    assert_eq!(
        serde_json::to_value(&catalog).unwrap(),
        json!([{
            "model_id": "no-config",
            "team_id": null,
            "created_by": null,
            "deployment": {
                "litellm_params": {
                    "model": "auto_router/complexity_router",
                    "complexity_router_config": null,
                },
                "model_info": {"id": "no-config", "db_model": true},
            },
        }])
    );
}

struct SnapshotCatalog {
    entries: Arc<Vec<AutoRouterCatalogEntry>>,
}

impl AutoRouterCatalog for SnapshotCatalog {
    fn catalog(&self) -> Option<Arc<Vec<AutoRouterCatalogEntry>>> {
        Some(Arc::clone(&self.entries))
    }
}

#[test]
fn catalog_trait_hands_out_the_entry_types() {
    let entries = Arc::new(vec![AutoRouterCatalogEntry {
        model_id: "m".to_owned(),
        team_id: Some("team-a".to_owned()),
        created_by: None,
        deployment: CatalogDeployment {
            litellm_params: CatalogLitellmParams {
                model: COMPLEXITY_MODEL.to_owned(),
                complexity_router_config: None,
            },
            model_info: CatalogModelInfo {
                id: "m".to_owned(),
                db_model: true,
            },
        },
    }]);
    let catalog: Arc<dyn AutoRouterCatalog> = Arc::new(SnapshotCatalog {
        entries: Arc::clone(&entries),
    });
    let handed = catalog.catalog().unwrap();
    assert_eq!(handed.as_ref(), entries.as_ref());
}
