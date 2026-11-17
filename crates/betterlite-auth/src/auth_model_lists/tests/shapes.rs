use betterlite_models::decisions::Json;

use super::{FakeProviders, object, text};
use crate::auth_model_lists::credential::CredentialSnapshot;
use crate::auth_model_lists::model_info::expand_wildcard_deployments;
use crate::auth_model_lists::parameters::{ModelParameters, ParameterError};
use crate::auth_model_lists::wildcard::ExpansionContext;

fn parsed(entries: &[(&str, Json)]) -> Result<ModelParameters, ParameterError> {
    ModelParameters::parse(&object(entries))
}

#[test]
fn credential_field_shapes_follow_pydantic() {
    parsed(&[("model", text("openai/*")), ("api_key", text("key"))]).unwrap();
    assert!(matches!(
        parsed(&[("model", text("openai/*")), ("api_key", Json::Bool(true))]),
        Err(ParameterError::Field { field: "api_key" })
    ));
    assert!(matches!(
        parsed(&[
            ("model", text("openai/*")),
            ("aws_session_tags", text("tag"))
        ]),
        Err(ParameterError::Field {
            field: "aws_session_tags"
        })
    ));
    parsed(&[
        ("model", text("openai/*")),
        ("aws_session_tags", Json::List(vec![text("tag")])),
        ("bedrock_tags", Json::List(vec![text("tag")])),
    ])
    .unwrap();
    parsed(&[
        ("model", text("openai/*")),
        ("vertex_credentials", text("json")),
    ])
    .unwrap();
    parsed(&[
        ("model", text("openai/*")),
        (
            "vertex_credentials",
            Json::Object(object(&[("type", text("service_account"))])),
        ),
    ])
    .unwrap();
    assert!(matches!(
        parsed(&[
            ("model", text("openai/*")),
            ("vertex_credentials", Json::List(Vec::new())),
        ]),
        Err(ParameterError::Field {
            field: "vertex_credentials"
        })
    ));
    assert!(matches!(
        parsed(&[
            ("model", text("openai/*")),
            ("litellm_credential_name", Json::Bool(false)),
        ]),
        Err(ParameterError::Field {
            field: "litellm_credential_name"
        })
    ));
    parsed(&[
        ("model", text("openai/*")),
        ("litellm_credential_name", Json::Null),
    ])
    .unwrap();
    parsed(&[
        ("model", text("openai/*")),
        ("litellm_credential_name", text("named")),
    ])
    .unwrap();
}

#[test]
fn concrete_slashed_name_and_star_over_concrete_model_kept() {
    let providers = FakeProviders::new(&[], &[("azure", &["gpt-4o-azure"])]);
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    let deployments = vec![
        object(&[
            ("model_name", text("openai/gpt-4o")),
            (
                "litellm_params",
                Json::Object(object(&[("model", text("azure/gpt-4o"))])),
            ),
        ]),
        object(&[
            ("model_name", text("*")),
            (
                "litellm_params",
                Json::Object(object(&[("model", text("azure/gpt-4o"))])),
            ),
        ]),
    ];
    assert_eq!(
        expand_wildcard_deployments(&deployments, &context),
        deployments,
        "a concrete name carrying a slash and a star over a concrete model both stay unexpanded"
    );
    assert!(
        providers.records().is_empty(),
        "a row that stays concrete never reaches the provider lookup"
    );
}
