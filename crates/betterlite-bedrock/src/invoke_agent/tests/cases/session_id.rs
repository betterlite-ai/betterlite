use betterlite_http::error::HttpError;

use crate::invoke_agent::error::InvokeAgentRequestError;
use crate::invoke_agent::identifiers::session_id as compute_session_id;
use crate::invoke_agent::tests::support::{SESSION_UUID, generated_uuid, params};
use crate::invoke_agent::url::{CompleteUrl, InvokeEnvironment, complete_url};
use crate::model_routes::ParamMap;

const MODEL: &str = "agent/L1RT58GYRW/MFPSBCXYTW";
const ENDPOINT: &str = "https://bedrock-agent-runtime.us-west-2.amazonaws.com";

pub(super) fn session_id() {
    assert_generated_or_supplied();
    assert_encoded_segment();
    assert_not_written_back();
}

fn try_url(optional_params: &ParamMap) -> Result<String, InvokeAgentRequestError> {
    complete_url(CompleteUrl {
        api_base: None,
        model: MODEL,
        optional_params,
        environment: InvokeEnvironment::default(),
        generated_session_id: generated_uuid(),
    })
}

fn assert_generated_or_supplied() {
    assert_eq!(
        compute_session_id(&params("{}"), generated_uuid()).unwrap(),
        SESSION_UUID
    );
    assert_eq!(
        compute_session_id(&params(r#"{"sessionID":null}"#), generated_uuid()).unwrap(),
        SESSION_UUID,
        "null takes the generated id"
    );
    assert_eq!(
        compute_session_id(&params(r#"{"sessionID":""}"#), generated_uuid()).unwrap(),
        SESSION_UUID,
        "a falsey explicit id generates a fresh one"
    );
    assert_eq!(
        compute_session_id(&params(r#"{"sessionID":"session-1"}"#), generated_uuid()).unwrap(),
        "session-1"
    );
    assert_eq!(
        compute_session_id(&params(r#"{"sessionID":" "}"#), generated_uuid()).unwrap(),
        " ",
        "a space is a truthy id"
    );
    for optional in [
        r#"{"sessionID":7}"#,
        r#"{"sessionID":0}"#,
        r#"{"sessionID":true}"#,
        r#"{"sessionID":false}"#,
        r#"{"sessionID":[]}"#,
        r#"{"sessionID":{}}"#,
    ] {
        assert!(
            matches!(
                compute_session_id(&params(optional), generated_uuid()),
                Err(InvokeAgentRequestError::WrongType { field: "sessionID" })
            ),
            "{optional}"
        );
    }
}

fn assert_encoded_segment() {
    assert_eq!(
        try_url(&params("{}")).unwrap(),
        format!(
            "{ENDPOINT}/agents/L1RT58GYRW/agentAliases/MFPSBCXYTW/sessions/{SESSION_UUID}/text"
        )
    );
    assert_eq!(
        try_url(&params(r#"{"sessionID":"a b/c?d"}"#)).unwrap(),
        format!("{ENDPOINT}/agents/L1RT58GYRW/agentAliases/MFPSBCXYTW/sessions/a%20b%2Fc%3Fd/text")
    );
    assert!(matches!(
        try_url(&params(r#"{"sessionID":".."}"#)),
        Err(InvokeAgentRequestError::Segment(HttpError::DotSegment { field_name }))
            if field_name == "session_id"
    ));
}

fn assert_not_written_back() {
    let supplied = params(r#"{"sessionID":"session-1","temperature":0.5}"#);
    let before = supplied.clone();
    drop(try_url(&supplied));
    assert_eq!(supplied, before, "a supplied id is not written back");
    let generated = params(r#"{"temperature":0.5}"#);
    let before_generated = generated.clone();
    drop(try_url(&generated));
    assert_eq!(
        generated, before_generated,
        "a generated id is not written back"
    );
}
