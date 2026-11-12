use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use indexmap::IndexSet;

use crate::bedrock_dispatch_contract::{
    BedrockAwsSettings, BedrockDispatchError, BedrockOperation, BedrockPlanRequest, dispatch,
};

fn assert_unsupported(operation: &BedrockOperation, name: &str) {
    let error = dispatch(operation).expect_err("a family without an installed operation refuses");
    assert_eq!(
        error.to_string(),
        format!("betterlite does not run the {name} operation through the bedrock provider")
    );
    match error {
        BedrockDispatchError::Unsupported { operation: found } => assert_eq!(found, name),
        BedrockDispatchError::Route(_)
        | BedrockDispatchError::Endpoint(_)
        | BedrockDispatchError::Encode { .. } => {
            panic!("a native family answered a rewritten error");
        }
    }
}

#[test]
fn bedrock_dispatch_contract_unsupported_operation() {
    assert_unsupported(&BedrockOperation::Chat, "chat");
    assert_unsupported(&BedrockOperation::Other, "other endpoints");
    assert_unsupported(&BedrockOperation::Services, "services");
    assert_unsupported(&BedrockOperation::Image, "image generation");
    assert_unsupported(&BedrockOperation::Agent, "agent");
    assert_unsupported(&BedrockOperation::Interactive, "interactive");

    let request = BedrockPlanRequest {
        model: String::from("anthropic.claude-3-haiku-20240307-v1:0"),
        model_id: None,
        aws_region_name: Some(String::from("us-east-1")),
        api_base: None,
        aws_bedrock_runtime_endpoint: None,
        settings: BedrockAwsSettings::default(),
        request_params: ParamMap::new(),
        additional_drop_params: Vec::new(),
        price_map: ParamMap::from([(
            String::from("openai.gpt-5.6-other"),
            ParamValue::Text(String::from("kept")),
        )]),
        converse_models: IndexSet::new(),
        request_metadata_owned: false,
    };
    let plan = dispatch(&BedrockOperation::Plan(Box::new(request)))
        .expect("the plan operation stays installed next to the unsupported families");
    assert_eq!(plan.region, "us-east-1");
}
