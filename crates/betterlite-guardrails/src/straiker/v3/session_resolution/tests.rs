#![cfg(test)]

use rstest::rstest;
use sha2::{Digest as _, Sha256};

use super::session_id;
use crate::straiker::v3::fixtures::{context, identity, object};

fn derived(seed: &str) -> String {
    let digest = hex::encode(Sha256::digest(seed.as_bytes()));
    format!("litellm-{}", digest.get(..32).unwrap())
}

const CHAT: &str = r#"{"system":"rules","messages":[{"role":"user","content":"hi"}]}"#;
const CLIENT_SESSION: &str =
    r#"{"proxy_server_request":{"headers":{"X-Claude-Code-Session-Id":" from-client "}}}"#;

#[rstest]
#[case::client_header_wins(CLIENT_SESSION, Some("meta"), Some("alice"), CHAT, "from-client".to_owned())]
#[case::litellm_session_second("{}", Some("meta"), Some("alice"), CHAT, "meta".to_owned())]
#[case::empty_litellm_session_is_unset(
    "{}",
    Some(""),
    Some("alice"),
    CHAT,
    derived("alice\0rules\0hi")
)]
#[case::derived_from_principal_and_conversation(
    "{}",
    None,
    Some("alice"),
    CHAT,
    derived("alice\0rules\0hi")
)]
#[case::another_principal_another_session("{}", None, Some("bob"), CHAT, derived("bob\0rules\0hi"))]
#[case::no_principal("{}", None, None, CHAT, derived("\0rules\0hi"))]
#[case::system_message_preamble(
    "{}",
    None,
    None,
    r#"{"messages":[{"role":"system","content":"rules"},{"role":"user","content":"hi"}]}"#,
    derived("\0rules\0hi")
)]
#[case::first_text_alone("{}", None, None, r#"{"prompt":"hi"}"#, derived("\0\0hi"))]
#[case::system_alone("{}", None, None, r#"{"system":"rules"}"#, derived("\0rules\0"))]
fn resolves_session(
    #[case] request_data: &str,
    #[case] metadata_session: Option<&str>,
    #[case] principal: Option<&str>,
    #[case] body: &str,
    #[case] expected: String,
) {
    let resolved = session_id(
        &context(metadata_session),
        &identity(principal),
        &object(request_data),
        &object(body),
    )
    .unwrap();
    assert_eq!(resolved, Some(expected));
}

#[rstest]
#[case::nothing_to_group_on("{}")]
#[case::empty_conversation(r#"{"messages":[],"system":null}"#)]
fn derives_nothing_without_a_conversation(#[case] body: &str) {
    let resolved = session_id(
        &context(None),
        &identity(Some("alice")),
        &object("{}"),
        &object(body),
    );
    assert_eq!(resolved.unwrap(), None);
}
