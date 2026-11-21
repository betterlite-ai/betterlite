#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::api_boundaries::ApiLimits;

use self::support::{
    FUZZ_SALT, SALT, advertised, at, capture, data, fixture, json_of, lifecycle_event, managed_id,
    principal, response, stream_event, text_of, unsealed,
};
use super::error::{Error, Forbidden};
use super::owned_id::{
    DecodedResponseId, OwnedIdCipher, decode_owned_response_id, decode_owned_response_id_text,
};
use super::{
    ADDRESSED_RESPONSE_ID_FIELD, CallOutcome, PROXY_HOOKS, ProxyHook, ResponsesCall,
    ResponsesIdSecurity,
};

const UNMANAGED_DETAIL: &str = "Forbidden. This response id was not issued by this proxy, so the proxy cannot tell who owns it. To let keys address responses this proxy did not issue, set general_settings::allow_unmanaged_response_ids to True in the config.yaml file.";
const USER_DETAIL: &str = "Forbidden. The response id is not associated with the user, who this key belongs to. To disable this security feature, set general_settings::disable_responses_id_security to True in the config.yaml file.";
const TEAM_DETAIL: &str = "Forbidden. The response id is not associated with the team, who this key belongs to. To disable this security feature, set general_settings::disable_responses_id_security to True in the config.yaml file.";
const NO_SIGNING_KEY: &str = "Response ID encryption is enabled but no signing key is configured. Please set LITELLM_SALT_KEY environment variable or configure a master_key. Skipping response ID encryption. See: https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key";

#[test]
fn same_user_team() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);
    let advertised_id = managed_id(&cipher, "resp_original", "alice", "team-a");
    let mut payload = data(vec![("previous_response_id", advertised_id.as_str())]);

    let outcome = policy
        .authorize_call(Some(ResponsesCall::Create), &mut payload, &alice)
        .unwrap();

    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "resp_original".to_owned(),
            addressed_id: advertised_id.clone(),
        }
    );
    assert_eq!(
        payload.get("previous_response_id"),
        Some(&Json::Text("resp_original".to_owned()))
    );
    assert_eq!(
        payload.get(ADDRESSED_RESPONSE_ID_FIELD),
        Some(&Json::Text(advertised_id.clone()))
    );
    assert_eq!(
        unsealed(&cipher, &advertised_id),
        "litellm_proxy:responses_api:response_id:resp_original;user_id:alice;team_id:team-a"
    );
}

#[test]
fn cross_user_forbidden() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);
    let bob = principal(Some("bob"), Some("team-b"), &[]);
    let no_user = principal(None, Some("team-a"), &[]);
    let advertised_id = managed_id(&cipher, "resp_original", "alice", "team-a");

    for caller in [&bob, &no_user] {
        let mut payload = data(vec![("response_id", advertised_id.as_str())]);
        let before = payload.clone();
        let error = policy
            .authorize_call(Some(ResponsesCall::Get), &mut payload, caller)
            .unwrap_err();
        assert!(matches!(error, Error::Forbidden(Forbidden::User)));
        assert_eq!(error.status_code(), 403);
        assert_eq!(error.to_string(), USER_DETAIL);
        assert_eq!(payload, before);
    }

    // the user check runs before the team check, so a wrong team alone gives the team detail
    let alice_wrong_team = principal(Some("alice"), Some("team-b"), &[]);
    let error = policy
        .authorize_call(
            Some(ResponsesCall::Delete),
            &mut data(vec![("response_id", advertised_id.as_str())]),
            &alice_wrong_team,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Forbidden(Forbidden::User)));
}

#[test]
fn cross_team_forbidden() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice_other_team = principal(Some("alice"), Some("team-b"), &[]);
    let alice_no_team = principal(Some("alice"), None, &[]);
    let advertised_id = managed_id(&cipher, "resp_original", "alice", "team-a");

    for caller in [&alice_other_team, &alice_no_team] {
        let mut payload = data(vec![("response_id", advertised_id.as_str())]);
        let before = payload.clone();
        let error = policy
            .authorize_call(Some(ResponsesCall::Cancel), &mut payload, caller)
            .unwrap_err();
        assert!(matches!(error, Error::Forbidden(Forbidden::Team)));
        assert_eq!(error.status_code(), 403);
        assert_eq!(error.to_string(), TEAM_DETAIL);
        assert_eq!(payload, before);
    }

    // an owner id with no team id never reaches the team check
    let no_team_owner = managed_id(&cipher, "resp_original", "alice", "");
    let outcome = policy
        .authorize_call(
            Some(ResponsesCall::ListInputItems),
            &mut data(vec![("response_id", no_team_owner.as_str())]),
            &alice_other_team,
        )
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "resp_original".to_owned(),
            addressed_id: no_team_owner.clone(),
        }
    );
}

#[test]
fn proxy_admin_unmanaged() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let admin = principal(Some("root"), Some("team-z"), &["platform_admin"]);
    let viewer = principal(Some("root"), Some("team-z"), &["platform_viewer"]);

    for unmanaged_id in ["resp_foreign", "chatcmpl-1"] {
        let mut payload = data(vec![("response_id", unmanaged_id)]);
        let outcome = policy
            .authorize_call(Some(ResponsesCall::Delete), &mut payload, &admin)
            .unwrap();
        assert_eq!(
            outcome,
            CallOutcome::Authorized {
                routing_id: unmanaged_id.to_owned(),
                addressed_id: unmanaged_id.to_owned(),
            }
        );
        assert_eq!(
            payload.get("response_id"),
            Some(&Json::Text(unmanaged_id.to_owned()))
        );
    }

    // an admin also reaches a managed id owned by someone else
    let advertised_id = managed_id(&cipher, "resp_original", "alice", "team-a");
    let outcome = policy
        .authorize_call(
            Some(ResponsesCall::Get),
            &mut data(vec![("response_id", advertised_id.as_str())]),
            &admin,
        )
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "resp_original".to_owned(),
            addressed_id: advertised_id,
        }
    );

    // only PROXY_ADMIN is exempt, not the view-only role
    let error = policy
        .authorize_call(
            Some(ResponsesCall::Get),
            &mut data(vec![("response_id", "resp_foreign")]),
            &viewer,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Forbidden(Forbidden::Unmanaged)));
}

#[test]
fn no_signing_key_debug_fallback() {
    let (settings, cipher) = fixture(false, false, None, 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);

    // without a key unmanaged ids pass, and the early return logs nothing
    let mut payload = data(vec![("response_id", "chatcmpl-1")]);
    let (outcome, logs) = capture(|| {
        policy
            .authorize_call(Some(ResponsesCall::Get), &mut payload, &alice)
            .unwrap()
    });
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "chatcmpl-1".to_owned(),
            addressed_id: "chatcmpl-1".to_owned(),
        }
    );
    assert!(logs.is_empty());

    // encrypting a nonstream response logs the exact fallback and leaves the id alone
    let mut body = response(r#"{"id":"resp_plain","created_at":0,"output":[]}"#);
    let (result, logs) = capture(|| policy.encrypt_response(&mut body, &alice));
    result.unwrap();
    assert_eq!(body.id, "resp_plain");
    assert_eq!(logs, vec![NO_SIGNING_KEY.to_owned()]);

    // a stream chunk on a create route logs it too and leaves the cache empty
    let mut event = lifecycle_event("resp_plain");
    let mut cache = BTreeMap::new();
    let (result, logs) = capture(|| {
        policy
            .rewrite_stream_chunk(Some("/v1/responses"), &mut event, &alice, &mut cache)
            .unwrap()
    });
    assert_eq!(
        text_of(at(&json_of(&event), &["response", "id"])),
        "resp_plain"
    );
    assert_eq!(logs, vec![NO_SIGNING_KEY.to_owned()]);
    assert!(cache.is_empty());
}

#[test]
fn stream_id_consistency() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);
    let mut cache = BTreeMap::new();

    let mut first = lifecycle_event("resp_stream");
    let mut second = lifecycle_event("resp_stream");
    policy
        .rewrite_stream_chunk(Some("/responses"), &mut first, &alice, &mut cache)
        .unwrap();
    policy
        .rewrite_stream_chunk(Some("/v1/responses"), &mut second, &alice, &mut cache)
        .unwrap();

    let first_id = text_of(at(&json_of(&first), &["response", "id"])).to_owned();
    let second_id = text_of(at(&json_of(&second), &["response", "id"])).to_owned();
    assert_eq!(first_id, second_id);
    assert_eq!(cache.len(), 1);
    let payload =
        "litellm_proxy:responses_api:response_id:resp_stream;user_id:alice;team_id:team-a";
    assert_eq!(unsealed(&cipher, &first_id), payload);

    // a second request starts a fresh cipher and cache: new nonces, same payload
    let (fresh_settings, fresh_cipher) = fixture(false, false, Some(SALT), 1_000);
    let fresh_policy = ResponsesIdSecurity {
        settings: &fresh_settings,
        cipher: &fresh_cipher,
        limits: ApiLimits::default(),
    };
    let mut fresh = lifecycle_event("resp_stream");
    let mut fresh_cache = BTreeMap::new();
    fresh_policy
        .rewrite_stream_chunk(Some("/responses"), &mut fresh, &alice, &mut fresh_cache)
        .unwrap();
    let fresh_id = text_of(at(&json_of(&fresh), &["response", "id"])).to_owned();
    assert_ne!(fresh_id, first_id);
    assert_eq!(unsealed(&fresh_cipher, &fresh_id), payload);
}

#[derive(Clone, Copy, Debug)]
enum IdShape {
    PrefixBeforeCiphertext,
    CiphertextBeforeSecondSplit,
    NoSplitToken,
    TwoFields,
    OneField,
    DecryptedWithoutManagedPrefix,
    DecryptionReturnsNull,
}

struct NullCipher;

impl OwnedIdCipher for NullCipher {
    fn decrypt_id(&self, _ciphertext: &str) -> Option<String> {
        None
    }
}

#[rstest]
#[case::prefix_before_ciphertext(IdShape::PrefixBeforeCiphertext)]
#[case::ciphertext_before_second_split_token(IdShape::CiphertextBeforeSecondSplit)]
#[case::no_resp_split_token(IdShape::NoSplitToken)]
#[case::two_fields(IdShape::TwoFields)]
#[case::one_field(IdShape::OneField)]
#[case::decrypted_without_managed_prefix(IdShape::DecryptedWithoutManagedPrefix)]
#[case::decryption_returns_null(IdShape::DecryptionReturnsNull)]
fn malformed_owned_id_payload(#[case] shape: IdShape) {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);
    let owned = managed_id(&cipher, "resp_test", "alice", "team-a");
    let wire = match shape {
        IdShape::PrefixBeforeCiphertext => format!("prefix{owned}"),
        IdShape::CiphertextBeforeSecondSplit => format!("{owned}resp_ignored"),
        IdShape::NoSplitToken => "plain-id".to_owned(),
        IdShape::TwoFields => advertised(
            &cipher,
            "litellm_proxy:responses_api:response_id:resp_test;user_id:alice",
        ),
        IdShape::OneField => {
            advertised(&cipher, "litellm_proxy:responses_api:response_id:resp_test")
        }
        IdShape::DecryptedWithoutManagedPrefix => {
            advertised(&cipher, "just-some-text;user_id:alice;team_id:team-a")
        }
        IdShape::DecryptionReturnsNull => "resp_whatever".to_owned(),
    };
    if let IdShape::DecryptionReturnsNull = shape {
        // a decryptor that answers null is unmanaged, as the Python None return is
        let decoded =
            decode_owned_response_id_text(&wire, &NullCipher, ApiLimits::default()).unwrap();
        assert_eq!(decoded, DecodedResponseId::Unmanaged);
        return;
    }
    let mut payload = data(vec![("response_id", wire.as_str())]);
    let before = payload.clone();
    let outcome = policy.authorize_call(Some(ResponsesCall::Get), &mut payload, &alice);

    match shape {
        IdShape::PrefixBeforeCiphertext | IdShape::CiphertextBeforeSecondSplit => {
            // both shapes read the segment between the first and second resp_ and recover resp_test
            assert_eq!(
                outcome.unwrap(),
                CallOutcome::Authorized {
                    routing_id: "resp_test".to_owned(),
                    addressed_id: wire.clone(),
                }
            );
            assert_eq!(
                payload.get("response_id"),
                Some(&Json::Text("resp_test".to_owned()))
            );
        }
        IdShape::OneField => {
            // fewer than two fields falls back to the whole advertised id with absent owners
            assert_eq!(
                outcome.unwrap(),
                CallOutcome::Authorized {
                    routing_id: wire.clone(),
                    addressed_id: wire.clone(),
                }
            );
        }
        IdShape::TwoFields => {
            let error = outcome.unwrap_err();
            assert!(matches!(error, Error::MalformedOwnedPayload { fields: 2 }));
            assert_eq!(error.status_code(), 500);
            assert_eq!(
                error.to_string(),
                "the managed response id payload has 2 fields; three are required"
            );
            assert_eq!(payload, before);
        }
        IdShape::NoSplitToken
        | IdShape::DecryptedWithoutManagedPrefix
        | IdShape::DecryptionReturnsNull => {
            let error = outcome.unwrap_err();
            assert!(matches!(error, Error::Forbidden(Forbidden::Unmanaged)));
            assert_eq!(error.status_code(), 403);
            assert_eq!(payload, before);
        }
    }
}

#[test]
fn default_unmanaged_denial() {
    assert_eq!(
        PROXY_HOOKS,
        [ProxyHook {
            name: "responses_id_security",
            opt_in: false,
        }]
    );
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    assert!(!settings.disable_responses_id_security);
    assert!(!settings.allow_unmanaged_response_ids);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);

    for unmanaged_id in ["resp_foreign", "chatcmpl-1", "resp_not-a-ciphertext"] {
        let mut payload = data(vec![("response_id", unmanaged_id)]);
        let before = payload.clone();
        let error = policy
            .authorize_call(Some(ResponsesCall::Get), &mut payload, &alice)
            .unwrap_err();
        assert!(matches!(error, Error::Forbidden(Forbidden::Unmanaged)));
        assert_eq!(error.status_code(), 403);
        assert_eq!(error.to_string(), UNMANAGED_DETAIL);
        assert_eq!(payload, before);
    }
}

#[test]
fn setting_overrides() {
    let bob = principal(Some("bob"), Some("team-b"), &[]);

    // allow_unmanaged_response_ids only opens the unmanaged path
    let (allow_settings, allow_cipher) = fixture(false, true, Some(SALT), 0);
    let allow = ResponsesIdSecurity {
        settings: &allow_settings,
        cipher: &allow_cipher,
        limits: ApiLimits::default(),
    };
    let outcome = allow
        .authorize_call(
            Some(ResponsesCall::Get),
            &mut data(vec![("response_id", "chatcmpl-1")]),
            &bob,
        )
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "chatcmpl-1".to_owned(),
            addressed_id: "chatcmpl-1".to_owned(),
        }
    );
    let owned = managed_id(&allow_cipher, "resp_original", "alice", "team-a");
    let error = allow
        .authorize_call(
            Some(ResponsesCall::Get),
            &mut data(vec![("response_id", owned.as_str())]),
            &bob,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Forbidden(Forbidden::User)));

    // disable_responses_id_security opens both paths and logs instead of failing
    let (disable_settings, disable_cipher) = fixture(true, false, Some(SALT), 0);
    let disable = ResponsesIdSecurity {
        settings: &disable_settings,
        cipher: &disable_cipher,
        limits: ApiLimits::default(),
    };
    let outcome = disable
        .authorize_call(
            Some(ResponsesCall::Get),
            &mut data(vec![("response_id", "chatcmpl-1")]),
            &bob,
        )
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "chatcmpl-1".to_owned(),
            addressed_id: "chatcmpl-1".to_owned(),
        }
    );
    let owned = managed_id(&disable_cipher, "resp_original", "alice", "team-a");
    let mut payload = data(vec![("response_id", owned.as_str())]);
    let (outcome, logs) = capture(|| {
        disable
            .authorize_call(Some(ResponsesCall::Get), &mut payload, &bob)
            .unwrap()
    });
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "resp_original".to_owned(),
            addressed_id: owned.clone(),
        }
    );
    assert_eq!(
        logs,
        vec![
            "Responses ID Security is disabled. User bob is accessing response id alice which is not associated with them."
                .to_owned()
        ]
    );

    // the same user in another team takes the team branch of the disabled log
    let alice_other_team = principal(Some("alice"), Some("team-b"), &[]);
    let (_, logs) = capture(|| {
        disable
            .check_access("alice", "team-a", &alice_other_team)
            .unwrap()
    });
    assert_eq!(
        logs,
        vec![
            "Responses ID Security is disabled. Response belongs to team team-a but user alice is accessing it with team id team-b."
                .to_owned()
        ]
    );

    // disabling also skips the advertised id rewrite, without logging the missing-key note
    let mut body = response(r#"{"id":"resp_plain","created_at":0,"output":[]}"#);
    let (result, logs) = capture(|| disable.encrypt_response(&mut body, &bob));
    result.unwrap();
    assert_eq!(body.id, "resp_plain");
    assert!(logs.is_empty());
    let mut event = lifecycle_event("resp_plain");
    let mut cache = BTreeMap::new();
    disable
        .rewrite_stream_chunk(Some("/responses"), &mut event, &bob, &mut cache)
        .unwrap();
    assert_eq!(
        text_of(at(&json_of(&event), &["response", "id"])),
        "resp_plain"
    );
    assert!(cache.is_empty());
}

#[test]
fn empty_owners() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let stranger = principal(Some("mallory"), Some("team-z"), &[]);

    // empty owner fields mean no owner check, whatever the caller
    let no_owners = advertised(
        &cipher,
        "litellm_proxy:responses_api:response_id:resp_original;user_id:;team_id:",
    );
    let outcome = policy
        .authorize_call(
            Some(ResponsesCall::Get),
            &mut data(vec![("response_id", no_owners.as_str())]),
            &stranger,
        )
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "resp_original".to_owned(),
            addressed_id: no_owners,
        }
    );

    // a payload with fewer than two fields keeps the whole advertised id as the routing id
    let one_field = advertised(
        &cipher,
        "litellm_proxy:responses_api:response_id:resp_original",
    );
    let outcome = policy
        .authorize_call(
            Some(ResponsesCall::Delete),
            &mut data(vec![("response_id", one_field.as_str())]),
            &stranger,
        )
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: one_field.clone(),
            addressed_id: one_field,
        }
    );
}

#[rstest]
#[case::create(ResponsesCall::Create, "previous_response_id")]
#[case::get(ResponsesCall::Get, "response_id")]
#[case::delete(ResponsesCall::Delete, "response_id")]
#[case::cancel(ResponsesCall::Cancel, "response_id")]
#[case::list_input_items(ResponsesCall::ListInputItems, "response_id")]
fn retained_addressed_id(#[case] call: ResponsesCall, #[case] field: &str) {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);
    let retained = managed_id(&cipher, "resp_retained", "alice", "team-a");
    let body_id = managed_id(&cipher, "resp_body", "alice", "team-a");

    let mut payload = data(vec![
        (ADDRESSED_RESPONSE_ID_FIELD, retained.as_str()),
        (field, body_id.as_str()),
    ]);
    let outcome = policy
        .authorize_call(Some(call), &mut payload, &alice)
        .unwrap();
    assert_eq!(
        outcome,
        CallOutcome::Authorized {
            routing_id: "resp_retained".to_owned(),
            addressed_id: retained.clone(),
        }
    );
    assert_eq!(
        payload.get(field),
        Some(&Json::Text("resp_retained".to_owned()))
    );
    assert_eq!(
        payload.get(ADDRESSED_RESPONSE_ID_FIELD),
        Some(&Json::Text(retained))
    );

    // an empty or non-string retained id falls back to the body field
    for retained_value in [Json::Text(String::new()), Json::Null, Json::Bool(true)] {
        let mut payload = data(vec![(field, body_id.as_str())]);
        payload.insert(ADDRESSED_RESPONSE_ID_FIELD.to_owned(), retained_value);
        let outcome = policy
            .authorize_call(Some(call), &mut payload, &alice)
            .unwrap();
        assert_eq!(
            outcome,
            CallOutcome::Authorized {
                routing_id: "resp_body".to_owned(),
                addressed_id: body_id.clone(),
            }
        );
        assert_eq!(
            payload.get(field),
            Some(&Json::Text("resp_body".to_owned()))
        );
        assert_eq!(
            payload.get(ADDRESSED_RESPONSE_ID_FIELD),
            Some(&Json::Text(body_id.clone()))
        );
    }

    // a missing, empty or non-string body field leaves the payload alone
    for empty in [
        Json::Text(String::new()),
        Json::Null,
        Json::Number(serde_json::Number::from(7_i32)),
    ] {
        let mut payload = data(vec![]);
        payload.insert(field.to_owned(), empty);
        let before = payload.clone();
        let outcome = policy
            .authorize_call(Some(call), &mut payload, &alice)
            .unwrap();
        assert_eq!(outcome, CallOutcome::Unchanged);
        assert_eq!(payload, before);
    }
    let mut payload = data(vec![]);
    assert_eq!(
        policy
            .authorize_call(Some(call), &mut payload, &alice)
            .unwrap(),
        CallOutcome::Unchanged
    );
    assert!(payload.is_empty());

    // a call type that is not a responses call returns nothing
    let mut payload = data(vec![(field, body_id.as_str())]);
    let before = payload.clone();
    assert_eq!(
        policy.authorize_call(None, &mut payload, &alice).unwrap(),
        CallOutcome::NotResponsesCall
    );
    assert_eq!(payload, before);
}

#[test]
fn response_rewrite_routes() {
    let (settings, cipher) = fixture(false, false, Some(SALT), 0);
    let policy = ResponsesIdSecurity {
        settings: &settings,
        cipher: &cipher,
        limits: ApiLimits::default(),
    };
    let alice = principal(Some("alice"), Some("team-a"), &[]);
    let payload = "litellm_proxy:responses_api:response_id:resp_plain;user_id:alice;team_id:team-a";

    // streaming rewrites only on the four create routes, and only a rewriteable event changes
    let routes = [
        (None, false),
        (Some("/responses"), true),
        (Some("/v1/responses"), true),
        (Some("/openai/responses"), true),
        (Some("/openai/v1/responses"), true),
        (Some("/chat/completions"), false),
        (Some("/v1/chat/completions"), false),
        (Some("/v1/responses/resp_test"), false),
        (Some("/v1/responses/"), false),
        (Some("/azure/v1/responses"), false),
        (Some("/openai"), false),
        (Some("/openai/"), false),
        (Some("/openai/v1/responses/extra"), false),
        (Some(" /v1/responses"), false),
        (Some("/v1/responses "), false),
    ];
    for (route, rewritten) in routes {
        let mut event = lifecycle_event("resp_plain");
        let mut cache = BTreeMap::new();
        policy
            .rewrite_stream_chunk(route, &mut event, &alice, &mut cache)
            .unwrap();
        let event_id = text_of(at(&json_of(&event), &["response", "id"])).to_owned();
        if rewritten {
            assert_ne!(event_id, "resp_plain", "{route:?}");
            assert_eq!(unsealed(&cipher, &event_id), payload, "{route:?}");
        } else {
            assert_eq!(event_id, "resp_plain", "{route:?}");
            assert!(cache.is_empty(), "{route:?}");
        }
    }

    // the nonstream rewrite has no route check: a resp_ top-level id is rewritten
    let mut top =
        response(r#"{"id":"resp_top","created_at":0,"output":[],"response":{"id":"resp_nested"}}"#);
    policy.encrypt_response(&mut top, &alice).unwrap();
    assert_ne!(top.id, "resp_top");
    assert_eq!(
        unsealed(&cipher, &top.id),
        "litellm_proxy:responses_api:response_id:resp_top;user_id:alice;team_id:team-a"
    );
    assert_eq!(
        text_of(at(&json_of(&top), &["response", "id"])),
        "resp_nested"
    );

    // without a resp_ top-level id the nested mapping is rewritten instead
    let mut nested =
        response(r#"{"id":"chat-1","created_at":0,"output":[],"response":{"id":"resp_nested"}}"#);
    policy.encrypt_response(&mut nested, &alice).unwrap();
    assert_eq!(nested.id, "chat-1");
    let nested_id = text_of(at(&json_of(&nested), &["response", "id"])).to_owned();
    assert_eq!(
        unsealed(&cipher, &nested_id),
        "litellm_proxy:responses_api:response_id:resp_nested;user_id:alice;team_id:team-a"
    );

    // no nested mapping, a non-mapping response, or a non-string nested id leaves the object alone
    let untouched = [
        r#"{"id":"chat-1","created_at":0,"output":[]}"#,
        r#"{"id":"chat-1","created_at":0,"output":[],"response":"text"}"#,
        r#"{"id":"chat-1","created_at":0,"output":[],"response":{"id":7}}"#,
        r#"{"id":"chat-1","created_at":0,"output":[],"response":null}"#,
    ];
    for source in untouched {
        let mut body = response(source);
        policy.encrypt_response(&mut body, &alice).unwrap();
        assert_eq!(json_of(&body), json_of(&response(source)), "{source}");
    }

    // a lifecycle event rewrites the typed response, with no prefix check
    let mut typed = stream_event(
        r#"{"type":"response.completed","response":{"id":"chat-1","created_at":0,"output":[]}}"#,
    );
    let mut cache = BTreeMap::new();
    policy
        .rewrite_stream_chunk(Some("/responses"), &mut typed, &alice, &mut cache)
        .unwrap();
    let typed_id = text_of(at(&json_of(&typed), &["response", "id"])).to_owned();
    assert_eq!(
        unsealed(&cipher, &typed_id),
        "litellm_proxy:responses_api:response_id:chat-1;user_id:alice;team_id:team-a"
    );

    // a top-level extra id that starts with resp_ wins over the typed response
    let mut both = stream_event(
        r#"{"type":"response.completed","response":{"id":"resp_typed","created_at":0,"output":[]},"id":"resp_extra"}"#,
    );
    let mut cache = BTreeMap::new();
    policy
        .rewrite_stream_chunk(Some("/responses"), &mut both, &alice, &mut cache)
        .unwrap();
    let both_json = json_of(&both);
    assert_eq!(text_of(at(&both_json, &["response", "id"])), "resp_typed");
    let extra_id = text_of(at(&both_json, &["id"])).to_owned();
    assert_eq!(
        unsealed(&cipher, &extra_id),
        "litellm_proxy:responses_api:response_id:resp_extra;user_id:alice;team_id:team-a"
    );

    // a generic event rewrites its top-level extra id, then falls back to a nested mapping
    let mut generic = stream_event(r#"{"type":"response.custom_thing","id":"resp_custom"}"#);
    let mut cache = BTreeMap::new();
    policy
        .rewrite_stream_chunk(Some("/v1/responses"), &mut generic, &alice, &mut cache)
        .unwrap();
    let generic_id = text_of(at(&json_of(&generic), &["id"])).to_owned();
    assert_eq!(
        unsealed(&cipher, &generic_id),
        "litellm_proxy:responses_api:response_id:resp_custom;user_id:alice;team_id:team-a"
    );
    let mut generic_nested = stream_event(
        r#"{"type":"response.custom_thing","id":"custom","response":{"id":"resp_custom"}}"#,
    );
    let mut cache = BTreeMap::new();
    policy
        .rewrite_stream_chunk(
            Some("/v1/responses"),
            &mut generic_nested,
            &alice,
            &mut cache,
        )
        .unwrap();
    let generic_json = json_of(&generic_nested);
    assert_eq!(text_of(at(&generic_json, &["id"])), "custom");
    let nested_custom = text_of(at(&generic_json, &["response", "id"])).to_owned();
    assert_eq!(
        unsealed(&cipher, &nested_custom),
        "litellm_proxy:responses_api:response_id:resp_custom;user_id:alice;team_id:team-a"
    );

    // a known non-lifecycle event keeps its extras, and rewrites a nested mapping there
    let mut delta = stream_event(
        r#"{"type":"response.output_text.delta","item_id":"item","output_index":0,"content_index":0,"delta":"x","response":{"id":"resp_delta"}}"#,
    );
    let mut cache = BTreeMap::new();
    policy
        .rewrite_stream_chunk(Some("/responses"), &mut delta, &alice, &mut cache)
        .unwrap();
    let delta_json = json_of(&delta);
    assert_eq!(text_of(at(&delta_json, &["delta"])), "x");
    let delta_id = text_of(at(&delta_json, &["response", "id"])).to_owned();
    assert_eq!(
        unsealed(&cipher, &delta_id),
        "litellm_proxy:responses_api:response_id:resp_delta;user_id:alice;team_id:team-a"
    );
}

#[test]
fn replays_corpus() {
    let (_, cipher) = fixture(false, false, Some(FUZZ_SALT), 0);
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/owned_response_id");
    let mut replayed = 0;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let decoded = decode_owned_response_id(&bytes, &cipher, ApiLimits::default());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the corpus names are UTF-8");
        match name {
            "managed-owners" => assert_eq!(
                decoded.unwrap(),
                DecodedResponseId::Managed {
                    original_id: "resp_orig".to_owned(),
                    user_id: "alice".to_owned(),
                    team_id: "team-a".to_owned(),
                }
            ),
            "managed-two-fields" => assert!(matches!(
                decoded,
                Err(Error::MalformedOwnedPayload { fields: 2 })
            )),
            "managed-one-field" => {
                assert_eq!(decoded.unwrap(), DecodedResponseId::Fallback)
            }
            "non-utf8" => assert!(matches!(decoded, Err(Error::NotUtf8))),
            "wrong-key" | "not-managed-payload" => {
                assert_eq!(decoded.unwrap(), DecodedResponseId::Unmanaged)
            }
            _ => drop(decoded),
        }
        replayed += 1;
    }
    assert!(replayed >= 10);
}
