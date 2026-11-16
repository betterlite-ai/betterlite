use serde::Deserialize;

#[derive(Deserialize)]
struct SetupEnvelope {
    #[serde(default)]
    setup: Option<ManualSetup>,
}

// only the keys turn detection needs, so a malformed sibling still leaves the manual flag readable
#[derive(Deserialize)]
struct ManualSetup {
    #[serde(default, rename = "realtimeInputConfig")]
    realtime_input_config: Option<ManualRealtimeInput>,
}

#[derive(Deserialize)]
struct ManualRealtimeInput {
    #[serde(default, rename = "automaticActivityDetection")]
    automatic_activity_detection: Option<ManualDetection>,
}

#[derive(Deserialize)]
struct ManualDetection {
    #[serde(default)]
    disabled: Option<bool>,
}

#[must_use]
pub fn manual_turn_detection(session_configuration_request: Option<&str>) -> bool {
    let Some(text) = session_configuration_request.filter(|text| !text.is_empty()) else {
        return false;
    };
    let Ok(envelope) = serde_json::from_str::<SetupEnvelope>(text) else {
        return false;
    };
    envelope
        .setup
        .and_then(|setup| setup.realtime_input_config)
        .and_then(|config| config.automatic_activity_detection)
        .and_then(|detection| detection.disabled)
        .unwrap_or(false)
}
