#![cfg(test)]

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::realtime_contracts::records::RealtimeClientSecretRequest;

use super::prepare::{
    PrepareError, PreparedSession, SessionGrantRequest, SessionModelGrant,
    prepare_client_secret_session,
};
use super::session::{
    DEFAULT_REALTIME_MODEL, DEFAULT_TRANSCRIPTION_MODEL, RealtimeSessionType, model_candidates,
    set_transcription_model,
};
use super::token::{
    DecodedRealtimeToken, RealtimeTokenFields, TokenDecodeLimits, decode_realtime_token_payload,
    encode_realtime_token_payload,
};

const LIMITS: TokenDecodeLimits = TokenDecodeLimits { max_bytes: 1 << 20 };

fn session(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn request(text: &str) -> RealtimeClientSecretRequest {
    serde_json::from_str(text).unwrap()
}

fn principal() -> Principal {
    serde_json::from_str(
        r#"{"principal_type":"human","subject":"u","auth_method":"api_key","user":{"id":"u"}}"#,
    )
    .unwrap()
}

fn locked<Value>(slot: &Mutex<Value>) -> MutexGuard<'_, Value> {
    match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct RouterMarker;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Asked {
    model: String,
    llm_model_list: Option<Vec<String>>,
    subject: String,
    router_present: bool,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("model access denied")]
struct Denied;

struct Grants {
    deny: Option<String>,
    asked: Mutex<Vec<Asked>>,
}

impl Grants {
    fn new() -> Self {
        Self {
            deny: None,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn denying(model: &str) -> Self {
        Self {
            deny: Some(model.to_owned()),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Asked> {
        locked(&self.asked).clone()
    }
}

#[async_trait]
impl SessionModelGrant<RouterMarker> for Grants {
    type Error = Denied;

    async fn authorize(
        &self,
        request: &SessionGrantRequest<'_, RouterMarker>,
    ) -> Result<(), Denied> {
        locked(&self.asked).push(Asked {
            model: request.model.to_owned(),
            llm_model_list: request.llm_model_list.map(<[String]>::to_vec),
            subject: request.principal.subject.clone(),
            router_present: request.router.is_some(),
        });
        if self.deny.as_deref() == Some(request.model) {
            return Err(Denied);
        }
        Ok(())
    }
}

fn models_asked(grants: &Grants) -> Vec<String> {
    grants
        .calls()
        .iter()
        .map(|call| call.model.clone())
        .collect()
}

#[test]
fn realtime_session_candidate_order() {
    assert_eq!(RealtimeSessionType::from_request(None).as_str(), "realtime");
    assert_eq!(
        RealtimeSessionType::from_request(Some("")).as_str(),
        "realtime"
    );
    assert_eq!(
        RealtimeSessionType::from_request(Some("Realtime")).as_str(),
        "realtime"
    );
    assert_eq!(
        RealtimeSessionType::from_request(Some("realtime")).as_str(),
        "realtime"
    );
    assert_eq!(
        RealtimeSessionType::from_request(Some("unknown")).as_str(),
        "realtime"
    );
    assert_eq!(
        RealtimeSessionType::from_request(Some("transcription")).as_str(),
        "transcription"
    );
    assert_eq!(
        RealtimeSessionType::from_request(Some("transcription ")).as_str(),
        "realtime"
    );
    assert_eq!(
        model_candidates(&session(
            r#"{"audio":{"input":{"transcription":{"model":"a"}}},"input_audio_transcription":{"model":"b"}}"#
        )),
        ["a", "b"]
    );
    assert_eq!(
        model_candidates(&session(
            r#"{"audio":{"input":{"transcription":{"model":"a"}}},"input_audio_transcription":{"model":"a"}}"#
        )),
        ["a"]
    );
}

#[test]
fn realtime_candidate_filtering() {
    assert!(model_candidates(&session("{}")).is_empty());
    assert!(model_candidates(&session(
        r#"{"audio":{"input":{"transcription":{"model":""}}},"input_audio_transcription":{"model":""}}"#
    ))
    .is_empty());
    assert!(model_candidates(&session(
        r#"{"audio":{"input":{"transcription":{"model":7}}},"input_audio_transcription":{"model":null}}"#
    ))
    .is_empty());
    assert_eq!(
        model_candidates(&session(
            r#"{"audio":{"input":{"transcription":{"model":" "}}},"input_audio_transcription":{"model":"  "}}"#
        )),
        [" ", "  "]
    );
    assert_eq!(
        model_candidates(&session(
            r#"{"audio":"no","input_audio_transcription":{"model":"b"}}"#
        )),
        ["b"]
    );
}

#[test]
fn realtime_transcription_model_setter() {
    let mut untouched = session(r#"{"model":"top"}"#);
    set_transcription_model(&mut untouched, "a", false);
    assert_eq!(untouched, session(r#"{"model":"top"}"#));

    let mut created = session(r#"{"model":"top"}"#);
    set_transcription_model(&mut created, "a", true);
    assert_eq!(
        created,
        session(r#"{"model":"top","audio":{"input":{"transcription":{"model":"a"}}}}"#)
    );

    let mut replaced = session(r#"{"audio":"text","input":"x"}"#);
    set_transcription_model(&mut replaced, "a", true);
    assert_eq!(
        replaced,
        session(r#"{"audio":{"input":{"transcription":{"model":"a"}}},"input":"x"}"#)
    );

    let mut replaced_input = session(r#"{"audio":{"input":"x","voice":"v"}}"#);
    set_transcription_model(&mut replaced_input, "a", true);
    assert_eq!(
        replaced_input,
        session(r#"{"audio":{"input":{"transcription":{"model":"a"}},"voice":"v"}}"#)
    );

    let mut replaced_transcription =
        session(r#"{"audio":{"input":{"format":"pcm","transcription":"x"}}}"#);
    set_transcription_model(&mut replaced_transcription, "a", true);
    assert_eq!(
        replaced_transcription,
        session(r#"{"audio":{"input":{"format":"pcm","transcription":{"model":"a"}}}}"#)
    );
}

#[test]
fn realtime_transcription_model_setter_keeps_siblings() {
    let mut both = session(
        r#"{"audio":{"voice":"v","input":{"format":"pcm","transcription":{"model":"old","language":"en"}}},"input_audio_transcription":{"model":"old","language":"en"},"model":"top"}"#,
    );
    set_transcription_model(&mut both, "a", true);
    assert_eq!(
        both,
        session(
            r#"{"audio":{"voice":"v","input":{"format":"pcm","transcription":{"model":"a","language":"en"}}},"input_audio_transcription":{"model":"a","language":"en"},"model":"top"}"#
        )
    );

    let mut flat_only = session(r#"{"input_audio_transcription":{"language":"en"}}"#);
    set_transcription_model(&mut flat_only, "a", true);
    assert_eq!(
        flat_only,
        session(r#"{"input_audio_transcription":{"language":"en","model":"a"}}"#)
    );

    let mut nested_only = session(
        r#"{"audio":{"input":{"transcription":{"model":"old","language":"en"}}},"model":"top"}"#,
    );
    set_transcription_model(&mut nested_only, "a", true);
    assert_eq!(
        nested_only,
        session(
            r#"{"audio":{"input":{"transcription":{"model":"a","language":"en"}}},"model":"top"}"#
        )
    );

    let mut none = session(r#"{"audio":"legacy","input_audio_transcription":"legacy"}"#);
    set_transcription_model(&mut none, "a", false);
    assert_eq!(
        none,
        session(r#"{"audio":"legacy","input_audio_transcription":"legacy"}"#)
    );
}

#[tokio::test]
async fn authorized_realtime_session_selection() {
    let identity = principal();
    let listed = vec!["listed".to_owned()];
    let grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request("{}"),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(
        prepared,
        PreparedSession {
            model: DEFAULT_REALTIME_MODEL.to_owned(),
            session: None,
            session_type: RealtimeSessionType::Realtime,
        }
    );
    assert_eq!(
        grants.calls(),
        [Asked {
            model: DEFAULT_REALTIME_MODEL.to_owned(),
            llm_model_list: Some(listed.clone()),
            subject: "u".to_owned(),
            router_present: true,
        }]
    );
}

#[tokio::test]
async fn authorized_realtime_empty_session_gets_type() {
    let identity = principal();
    let listed = vec!["listed".to_owned()];
    let grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request(r#"{"session":{}}"#),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, DEFAULT_REALTIME_MODEL);
    assert_eq!(prepared.session, Some(session(r#"{"type":"realtime"}"#)));
    assert_eq!(models_asked(&grants), [DEFAULT_REALTIME_MODEL]);
}

#[tokio::test]
async fn authorized_realtime_session_model_wins_once() {
    let identity = principal();
    let listed = vec!["listed".to_owned()];
    let grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request(r#"{"session":{"type":"realtime","model":"session-model"},"model":"root-model"}"#),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, "session-model");
    assert_eq!(
        prepared.session,
        Some(session(r#"{"type":"realtime","model":"session-model"}"#))
    );
    assert_eq!(models_asked(&grants), ["session-model"]);
    assert!(grants.calls().iter().all(|call| call.subject == "u"
        && call.llm_model_list == Some(listed.clone())
        && call.router_present));
}

#[tokio::test]
async fn authorized_realtime_falls_back_to_root_then_default() {
    let identity = principal();
    let grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request(r#"{"session":{"model":""},"model":"root-model"}"#),
        &identity,
        None,
        None::<&RouterMarker>,
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, "root-model");
    assert_eq!(
        grants.calls(),
        [Asked {
            model: "root-model".to_owned(),
            llm_model_list: None,
            subject: "u".to_owned(),
            router_present: false,
        }]
    );

    let second_grants = Grants::new();
    let second_prepared = prepare_client_secret_session(
        &request(r#"{"model":""}"#),
        &identity,
        None,
        None::<&RouterMarker>,
        &second_grants,
    )
    .await
    .unwrap();
    assert_eq!(second_prepared.model, DEFAULT_REALTIME_MODEL);
}

#[tokio::test]
async fn authorized_session_dump_covers_every_field() {
    let identity = principal();
    let grants = Grants::new();
    let text = r#"{"session":{"type":"realtime","model":"m","instructions":"i","audio":{"voice":"v"},"include":["a"],"max_output_tokens":7,"output_modalities":["text"],"tool_choice":"auto","tools":[{"type":"function"}],"tracing":"off","truncation":"auto","prompt":{"id":"p"},"future":{"a":1},"gone":null}}"#;
    let prepared = prepare_client_secret_session(
        &request(text),
        &identity,
        None,
        None::<&RouterMarker>,
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, "m");
    assert_eq!(
        prepared.session,
        Some(session(
            r#"{"type":"realtime","model":"m","instructions":"i","audio":{"voice":"v"},"include":["a"],"max_output_tokens":7,"output_modalities":["text"],"tool_choice":"auto","tools":[{"type":"function"}],"tracing":"off","truncation":"auto","prompt":{"id":"p"},"future":{"a":1}}"#
        ))
    );

    let wordy = prepare_client_secret_session(
        &request(r#"{"session":{"max_output_tokens":"word"}}"#),
        &identity,
        None,
        None::<&RouterMarker>,
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(
        wordy.session,
        Some(session(r#"{"type":"realtime","max_output_tokens":"word"}"#))
    );
}

#[tokio::test]
async fn authorized_transcription_session_selection() {
    let identity = principal();
    let listed = vec!["listed".to_owned()];
    let text = r#"{"session":{"type":"transcription","model":"session-model","audio":{"input":{"transcription":{"model":"allowed","language":"en"}}},"input_audio_transcription":{"model":"forbidden","language":"en"},"instructions":"hi"}}"#;
    let grants = Grants::denying("forbidden");
    let failure = prepare_client_secret_session(
        &request(text),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &grants,
    )
    .await
    .unwrap_err();
    assert!(matches!(failure, PrepareError::Grant(Denied)));
    assert_eq!(models_asked(&grants), ["allowed", "forbidden"]);

    let second_grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request(text),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &second_grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, "allowed");
    assert_eq!(
        prepared.session,
        Some(session(
            r#"{"type":"transcription","audio":{"input":{"transcription":{"model":"allowed","language":"en"}}},"input_audio_transcription":{"model":"allowed","language":"en"},"instructions":"hi"}"#
        ))
    );
    assert_eq!(models_asked(&grants), ["allowed", "forbidden"]);
}

#[tokio::test]
async fn authorized_transcription_session_fallbacks() {
    let identity = principal();
    let listed = vec!["listed".to_owned()];
    let grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request(
            r#"{"session":{"type":"transcription","model":"session-model"},"model":"root-model"}"#,
        ),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, "session-model");
    assert_eq!(
        prepared.session,
        Some(session(
            r#"{"type":"transcription","audio":{"input":{"transcription":{"model":"session-model"}}}}"#
        ))
    );
    assert_eq!(models_asked(&grants), ["session-model", "root-model"]);
    assert!(grants.calls().iter().all(|call| call.subject == "u"
        && call.llm_model_list == Some(listed.clone())
        && call.router_present));
}

#[tokio::test]
async fn authorized_transcription_default_creates_transcription() {
    let identity = principal();
    let listed = vec!["listed".to_owned()];
    let grants = Grants::new();
    let prepared = prepare_client_secret_session(
        &request(r#"{"session":{"type":"transcription"}}"#),
        &identity,
        Some(&listed),
        Some(&RouterMarker),
        &grants,
    )
    .await
    .unwrap();
    assert_eq!(prepared.model, DEFAULT_TRANSCRIPTION_MODEL);
    assert_eq!(
        prepared.session,
        Some(session(
            r#"{"type":"transcription","audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}}}}"#
        ))
    );
    assert_eq!(models_asked(&grants), [DEFAULT_TRANSCRIPTION_MODEL]);
}

#[test]
fn realtime_token_encoding() {
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &Json::Text("ep".to_owned()),
            model_id: "m",
            user_id: None,
            team_id: Some(""),
            expires_at: &Json::Null,
            session_type: "realtime",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":"ep","model_id":"m","user_id":"","team_id":"","expires_at":null,"session_type":"realtime"}"#
    );
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &Json::Text("ep".to_owned()),
            model_id: "m",
            user_id: Some("u"),
            team_id: Some("t"),
            expires_at: &Json::Number(Number::from(5_i64)),
            session_type: "transcription",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":"ep","model_id":"m","user_id":"u","team_id":"t","expires_at":5,"session_type":"transcription"}"#
    );
}

#[test]
fn realtime_token_encoding_escapes() {
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &Json::Text("ep\u{e9}".to_owned()),
            model_id: "m\u{65e5}",
            user_id: Some("u\u{1f600}"),
            team_id: Some("\u{7f}"),
            expires_at: &Json::Null,
            session_type: "realtime",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":"ep\u00e9","model_id":"m\u65e5","user_id":"u\ud83d\ude00","team_id":"\u007f","expires_at":null,"session_type":"realtime"}"#
    );
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &Json::Text("a\"b\\c\n\t\u{1f}".to_owned()),
            model_id: "",
            user_id: None,
            team_id: None,
            expires_at: &Json::Null,
            session_type: "",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":"a\"b\\c\n\t\u001f","model_id":"","user_id":"","team_id":"","expires_at":null,"session_type":""}"#
    );
}

#[test]
fn realtime_token_encoding_passes_values_through() {
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &Json::Null,
            model_id: "m",
            user_id: None,
            team_id: None,
            expires_at: &Json::Null,
            session_type: "realtime",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":null,"model_id":"m","user_id":"","team_id":"","expires_at":null,"session_type":"realtime"}"#
    );
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &Json::Number(Number::from(7_i64)),
            model_id: "m",
            user_id: None,
            team_id: None,
            expires_at: &Json::Number(Number::from(7_i64)),
            session_type: "realtime",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":7,"model_id":"m","user_id":"","team_id":"","expires_at":7,"session_type":"realtime"}"#
    );
    let nested = Json::Object(IndexMap::from([(
        "a".to_owned(),
        Json::List(vec![Json::Bool(true)]),
    )]));
    assert_eq!(
        encode_realtime_token_payload(&RealtimeTokenFields {
            ephemeral_key: &nested,
            model_id: "m",
            user_id: None,
            team_id: None,
            expires_at: &Json::Null,
            session_type: "realtime",
        }),
        r#"{"v":"realtime_v1","ephemeral_key":{"a":[true]},"model_id":"m","user_id":"","team_id":"","expires_at":null,"session_type":"realtime"}"#
    );
}

fn decode(text: &str) -> Option<DecodedRealtimeToken> {
    decode_realtime_token_payload(text.as_bytes(), LIMITS)
}

fn decoded_value(token: &DecodedRealtimeToken) -> Json {
    serde_json::from_str(&serde_json::to_string(token).unwrap()).unwrap()
}

#[test]
fn realtime_token_legacy_decoding() {
    let rejected = [
        "ep-test",
        "{",
        "[]",
        "null",
        r#""realtime_v1""#,
        r#"{"v":"realtime_v2","ephemeral_key":"k","model_id":"m"}"#,
        r#"{"ephemeral_key":"k","model_id":"m"}"#,
        r#"{"v":null,"ephemeral_key":"k","model_id":"m"}"#,
        r#"{"v":"realtime_v1","ephemeral_key":null,"model_id":"m"}"#,
        r#"{"v":"realtime_v1","ephemeral_key":7,"model_id":"m"}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":7}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k"}"#,
        r#"{"v":"realtime_v1","model_id":"m"}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m"} x"#,
        "\u{feff}{}",
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":NaN}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":1e400}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":[1e400]}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":{"y":-1e400}}"#,
        r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":"\ud800"}"#,
    ];
    assert!(
        rejected.iter().all(|text| decode(text).is_none()),
        "a legacy or malformed token decoded"
    );
    assert!(
        decode_realtime_token_payload(
            b"{\"v\":\"realtime_v1\",\"ephemeral_key\":\"\xff\",\"model_id\":\"m\"}",
            LIMITS,
        )
        .is_none()
    );
}

#[test]
fn realtime_token_decoding_keeps_fields() {
    let minimal = r#"{"v":"realtime_v1","ephemeral_key":"","model_id":""}"#;
    let token = decode(minimal).unwrap();
    assert_eq!(token.version, "realtime_v1");
    assert_eq!(token.ephemeral_key, "");
    assert_eq!(token.model_id, "");
    assert!(token.fields.is_empty());
    assert_eq!(
        decoded_value(&token),
        serde_json::from_str::<Json>(minimal).unwrap()
    );

    let full = r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","user_id":7,"team_id":null,"expires_at":true,"session_type":3,"future":{"a":1}}"#;
    let full_token = decode(full).unwrap();
    assert_eq!(full_token.ephemeral_key, "k");
    assert_eq!(
        decoded_value(&full_token),
        serde_json::from_str::<Json>(full).unwrap()
    );

    assert_eq!(
        decode(
            r#"{"v":"realtime_v1","ephemeral_key":"first","ephemeral_key":"last","model_id":"m"}"#
        )
        .unwrap()
        .ephemeral_key,
        "last"
    );
}

#[test]
fn realtime_token_decode_limits() {
    let token = r#"{"v":"realtime_v1","ephemeral_key":"k","model_id":"m"}"#;
    let exact = TokenDecodeLimits {
        max_bytes: token.len(),
    };
    assert!(decode_realtime_token_payload(token.as_bytes(), exact).is_some());
    let one_short = TokenDecodeLimits {
        max_bytes: token.len() - 1,
    };
    assert!(decode_realtime_token_payload(token.as_bytes(), one_short).is_none());
    assert!(
        decode("  {\"v\":\"realtime_v1\",\"ephemeral_key\":\"k\",\"model_id\":\"m\"}  ").is_some()
    );
    let accepted = format!(
        r#"{{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":{}0{}}}"#,
        "[".repeat(126),
        "]".repeat(126)
    );
    assert!(decode(&accepted).is_some());
    let refused = format!(
        r#"{{"v":"realtime_v1","ephemeral_key":"k","model_id":"m","x":{}0{}}}"#,
        "[".repeat(127),
        "]".repeat(127)
    );
    assert!(decode(&refused).is_none());
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/realtime_token_payload");
    let decoded: Vec<Option<DecodedRealtimeToken>> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| decode_realtime_token_payload(&bytes, LIMITS))
        .collect();
    assert!(decoded.len() >= 10);
    assert!(decoded.iter().any(Option::is_some));
    assert!(decoded.iter().any(Option::is_none));
}
