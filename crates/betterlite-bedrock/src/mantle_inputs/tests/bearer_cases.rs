use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::mantle_inputs::chat_endpoint::{ChatEndpointParams, chat_endpoint_inputs};
use crate::mantle_inputs::credentials::resolve_bearer_token;
use crate::mantle_inputs::records::MantleGlobals;

use super::support::{empty_cost, exposed, globals, secret};

fn token(globals: &MantleGlobals, api_key: Option<&SecretString>) -> Option<String> {
    resolve_bearer_token(
        api_key,
        globals.mantle_api_key.as_ref(),
        globals.aws_bearer_token.as_ref(),
    )
    .map(|selected| selected.expose_secret().to_owned())
}

pub(super) fn mantle_bearer_precedence() {
    precedence_cases();
    empty_last_operand_cases();
    delegation_case();
}

fn precedence_cases() {
    let unset = globals(&[]);
    assert_eq!(token(&unset, None), None);
    assert_eq!(token(&unset, Some(&secret(""))), None);

    let both = globals(&[
        ("BEDROCK_MANTLE_API_KEY", "mantle-key"),
        ("AWS_BEARER_TOKEN_BEDROCK", "standard-key"),
    ]);
    assert_eq!(
        token(&both, Some(&secret("explicit-key"))),
        Some("explicit-key".to_owned())
    );
    // an empty explicit key falls through instead of disabling bearer auth, unlike B590
    assert_eq!(
        token(&both, Some(&secret(""))),
        Some("mantle-key".to_owned())
    );
    assert_eq!(token(&both, None), Some("mantle-key".to_owned()));

    let standard_only = globals(&[("AWS_BEARER_TOKEN_BEDROCK", "standard-key")]);
    assert_eq!(token(&standard_only, None), Some("standard-key".to_owned()));

    let blank_mantle = globals(&[
        ("BEDROCK_MANTLE_API_KEY", ""),
        ("AWS_BEARER_TOKEN_BEDROCK", "standard-key"),
    ]);
    assert_eq!(token(&blank_mantle, None), Some("standard-key".to_owned()));
}

fn empty_last_operand_cases() {
    // the last operand of LiteLLM's `or` chain is returned even when it is empty
    let blank_standard = globals(&[("AWS_BEARER_TOKEN_BEDROCK", "")]);
    assert_eq!(token(&blank_standard, None), Some(String::new()));
    assert_eq!(
        token(&blank_standard, Some(&secret(""))),
        Some(String::new())
    );
    let both_blank = globals(&[
        ("BEDROCK_MANTLE_API_KEY", ""),
        ("AWS_BEARER_TOKEN_BEDROCK", ""),
    ]);
    assert_eq!(token(&both_blank, None), Some(String::new()));
    let mantle_only_blank = globals(&[("BEDROCK_MANTLE_API_KEY", "")]);
    assert_eq!(token(&mantle_only_blank, None), None);
}

fn delegation_case() {
    let globals = globals(&[
        ("BEDROCK_MANTLE_API_KEY", "mantle-key"),
        ("AWS_BEARER_TOKEN_BEDROCK", "standard-key"),
    ]);
    let blank = secret("");
    let keyed = chat_endpoint_inputs(
        ChatEndpointParams {
            model: None,
            api_base: None,
            api_key: Some(&blank),
            aws_region_name: None,
        },
        &globals,
        &empty_cost(),
    )
    .unwrap();
    assert_eq!(exposed(keyed.api_key.as_ref()), Some("mantle-key"));
    assert_eq!(
        token(&globals, Some(&blank)),
        exposed(keyed.api_key.as_ref()).map(str::to_owned)
    );
}
