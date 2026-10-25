use betterlite_models::decisions::HiddenParams;
use betterlite_models::decisions::openai::DecisionResponse;
use betterlite_models::decisions::systemone::DecisionsResponse;

const PROVIDER_AND_COST: &str = r#"{"custom_llm_provider":"openai","cost":2}"#;

#[test]
fn keeps_hidden_params_out_of_systemone_json() {
    let json = r#"{"model":null,"answers":{},"usage":null,"hidden_params":{"leak":true}}"#;
    let mut response: DecisionsResponse = serde_json::from_str(json).unwrap();
    assert!(response.hidden_params.is_empty());
    let params: HiddenParams = serde_json::from_str(PROVIDER_AND_COST).unwrap();
    response.hidden_params.extend(params.clone());
    assert_eq!(response.hidden_params, params);
    assert_eq!(serde_json::to_string(&response).unwrap(), json);
}

#[test]
fn keeps_hidden_params_out_of_openai_json() {
    let usage = r#"{"input_tokens":0,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":0}"#;
    let json = format!(r#"{{"model":"judge","answers":[],"usage":{usage}}}"#);
    let mut response: DecisionResponse = serde_json::from_str(&json).unwrap();
    response
        .hidden_params
        .extend(serde_json::from_str::<HiddenParams>(PROVIDER_AND_COST).unwrap());
    assert_eq!(
        serde_json::to_string(&response.hidden_params).unwrap(),
        PROVIDER_AND_COST
    );
    assert_eq!(serde_json::to_string(&response).unwrap(), json);
}
