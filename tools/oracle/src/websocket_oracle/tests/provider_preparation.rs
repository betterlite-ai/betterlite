#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::{
    Arguments, DriverError, Frame, ServiceInput, ServiceTag, Services, TranscriptObservation,
};
use super::{compare_transcript, decode_ws_case, fixtures, limits, run_adapter, valid};
use crate::compare::helper::HelperOutput;
use crate::websocket_oracle::{Action, Direction, output_matches};

struct Preparation {
    url: String,
    query: Vec<(String, String)>,
    headers: BTreeMap<String, String>,
}

impl Preparation {
    fn frames(&self) -> Vec<Frame> {
        [
            self.url.clone(),
            serde_json::to_string(&self.query).unwrap(),
            serde_json::to_string(&self.headers).unwrap(),
        ]
        .into_iter()
        .map(|text| Frame {
            direction: Direction::ProviderToClient,
            action: Action::Expect,
            text: Some(text),
            binary: None,
            binary_data: None,
            barrier: None,
        })
        .collect()
    }
}

fn prepare(args: &Arguments, key: &SecretString) -> Preparation {
    let query: Vec<_> = args
        .query_params
        .clone()
        .unwrap_or_else(|| indexmap::IndexMap::from([("model".to_owned(), args.model.clone())]))
        .into_iter()
        .collect();
    let base = args
        .api_base
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut url = Url::parse(&base).unwrap();
    url.set_path("/v1/realtime");
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(&query);
    }
    let mut headers = BTreeMap::from([(
        "Authorization".to_owned(),
        format!("Bearer {}", key.expose_secret()),
    )]);
    if args.openai_beta_realtime {
        headers.insert("OpenAI-Beta".to_owned(), "realtime=v1".to_owned());
    }
    Preparation {
        url: url.to_string(),
        query,
        headers,
    }
}

fn preparation_adapter(args: &Arguments) -> Result<TranscriptObservation, DriverError> {
    let key = args.api_key.as_ref().ok_or_else(|| {
        DriverError::Transport("api_key is required for OpenAI realtime calls".to_owned())
    })?;
    let actual = prepare(args, key);
    run_adapter(args, |script| {
        script
            .frames
            .retain(|frame| frame.direction == Direction::ClientToProvider);
        script.frames.extend(actual.frames());
    })
}

#[rstest::rstest]
#[case::absent("provider-preparation", "wss://api.openai.com/v1/realtime?model=realtime-a", &[("model", "realtime-a")][..], false)]
#[case::empty("probe-empty", "wss://api.openai.com/v1/realtime", &[][..], false)]
#[case::http_ordered("probe-http", "ws://provider.test/v1/realtime?intent=transcription&model=realtime-a", &[("intent", "transcription"), ("model", "realtime-a")][..], false)]
#[case::beta("probe-beta", "wss://api.openai.com/v1/realtime?model=realtime-a", &[("model", "realtime-a")][..], true)]
fn pinned_observations(
    #[case] fixture: &str,
    #[case] url: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] beta: bool,
    #[values(0, 1, 2)] changed: usize,
) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/harness/websocket-oracle")
        .join(format!("{fixture}.yaml"));
    let mut case = decode_ws_case(&fs::read(path).unwrap(), &limits(), &fixtures()).unwrap();
    let mut headers = BTreeMap::from([("Authorization".to_owned(), "Bearer sk-test".to_owned())]);
    if beta {
        headers.insert("OpenAI-Beta".to_owned(), "realtime=v1".to_owned());
    }
    let expected = Preparation {
        url: url.to_owned(),
        query: pairs
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        headers,
    };
    assert_eq!(case.frames, expected.frames());
    let texts: Vec<_> = case.frames.iter().map(|frame| frame.text.clone()).collect();
    let mut output = python_output(&texts);
    output_matches(&case, &output).unwrap();
    output.value.as_mut().unwrap()["frames"][changed]["text"] =
        serde_json::Value::String("changed".to_owned());
    assert_eq!(
        output_matches(&case, &output),
        Err(format!("/provider/frames/{changed}/text"))
    );
    case.frames = decode_ws_case(valid().as_bytes(), &limits(), &fixtures())
        .unwrap()
        .frames;
    case.frames.truncate(1);
    case.frames.extend(expected.frames());
    case.input.frames = case.frames.clone();
    let input = ServiceInput {
        service: ServiceTag::RealtimeProbe,
        input: case.input.clone(),
    };
    let encoded = serde_json::to_value(input.to_wire()).unwrap();
    let restored: ServiceInput = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored.to_wire()).unwrap(), encoded);
    let mut registry = Services::default();
    registry
        .register_service(ServiceTag::RealtimeProbe, preparation_adapter)
        .unwrap();
    let mut observed = registry
        .run_service(restored.service, &restored.input)
        .unwrap();
    compare_transcript(&case, &observed).unwrap();
    observed
        .frames
        .iter_mut()
        .filter(|frame| frame.direction == Direction::ProviderToClient)
        .nth(changed)
        .unwrap()
        .text = Some("changed".to_owned());
    assert_eq!(
        compare_transcript(&case, &observed),
        Err(format!("/provider/frames/{changed}/text"))
    );
}

fn python_output(texts: &[Option<String>]) -> HelperOutput {
    let frames: Vec<_> = texts
        .iter()
        .map(|text| {
            let encoded = serde_json::to_string(text).unwrap();
            format!(r#"{{"direction":"provider_to_client","action":"expect","text":{encoded}}}"#)
        })
        .collect();
    let joined = frames.join(",");
    serde_json::from_str(&format!(
        r#"{{"ok":true,"value":{{"handshake":{{"status":101,"negotiated_subprotocol":null}},"frames":[{joined}],"close":null}}}}"#
    ))
    .unwrap()
}

#[test]
fn missing_key() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/harness/websocket-oracle/probe-missing-key.yaml");
    let case = decode_ws_case(&fs::read(path).unwrap(), &limits(), &fixtures()).unwrap();
    let error = |message: &str| -> HelperOutput {
        let encoded = serde_json::to_string(message).unwrap();
        serde_json::from_str(&format!(
            r#"{{"ok":false,"error":{{"class":"ValueError","status":null,"type":null,"param":null,"code":null,"message":{encoded}}}}}"#
        ))
        .unwrap()
    };
    output_matches(
        &case,
        &error("api_key is required for OpenAI realtime calls"),
    )
    .unwrap();
    assert_eq!(
        output_matches(&case, &error("changed")),
        Err("/error/message".to_owned())
    );
    assert_eq!(
        output_matches(&case, &python_output(&[])),
        Err("/ok".to_owned())
    );
    let mut registry = Services::default();
    registry
        .register_service(ServiceTag::RealtimeProbe, preparation_adapter)
        .unwrap();
    assert!(
        matches!(registry.run_service(ServiceTag::RealtimeProbe, &Arguments::default()),
        Err(DriverError::Transport(message)) if message == "api_key is required for OpenAI realtime calls")
    );
}
