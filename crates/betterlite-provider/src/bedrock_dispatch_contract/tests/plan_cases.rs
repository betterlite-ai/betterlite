use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use betterlite_bedrock::aws_endpoints::region::{LlmRegion, llm_region_name};
use betterlite_bedrock::aws_endpoints::runtime::{
    BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint,
};
use betterlite_bedrock::model_routes::{
    BedrockRoute, ModelRouteSelector, ParamMap, ParamValue, RouteError,
};
use indexmap::IndexSet;

use crate::bedrock_dispatch_contract::{
    BedrockAwsSettings, BedrockDispatchError, BedrockOperation, BedrockPlanRequest, dispatch,
};

const CLAUDE: &str = "anthropic.claude-3-haiku-20240307-v1:0";
const ENCODED_CLAUDE: &str = "anthropic.claude-3-haiku-20240307-v1:0";
const ARN: &str =
    "arn:aws:bedrock:us-west-2:123456789012:inference-profile/global.anthropic.claude";
const ENCODED_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aus-west-2%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude";
const OVERRIDE_ARN: &str =
    "arn:aws:bedrock:us-east-2:123456789012:inference-profile/global.anthropic.claude";
const ENCODED_OVERRIDE_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aus-east-2%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude";
const DEFAULT_ENDPOINT: &str = "https://bedrock-runtime.us-west-2.amazonaws.com";

struct Expected {
    operation: BedrockRoute,
    model_id: &'static str,
    region: &'static str,
    endpoint_url: &'static str,
    proxy_endpoint_url: &'static str,
}

fn plain(model: &str) -> BedrockPlanRequest {
    BedrockPlanRequest {
        model: model.to_owned(),
        model_id: None,
        aws_region_name: None,
        api_base: None,
        aws_bedrock_runtime_endpoint: None,
        settings: BedrockAwsSettings::default(),
        request_params: ParamMap::new(),
        additional_drop_params: Vec::new(),
        price_map: ParamMap::new(),
        converse_models: IndexSet::new(),
        request_metadata_owned: false,
    }
}

fn check(request: BedrockPlanRequest, expected: &Expected) {
    let plan = dispatch(&BedrockOperation::Plan(request.clone()))
        .expect("the plan composes the route and endpoint leaves");
    assert_eq!(plan.operation, expected.operation);
    assert_eq!(plan.model, request.model);
    assert_eq!(plan.model_id, expected.model_id);
    assert_eq!(plan.region, expected.region);
    assert_eq!(plan.endpoint_url, expected.endpoint_url);
    assert_eq!(plan.proxy_endpoint_url, expected.proxy_endpoint_url);
}

fn malformed_catalog_request() -> BedrockPlanRequest {
    let mut request = plain("openai.gpt-5.6-fixture");
    request.price_map.insert(
        String::from("openai.gpt-5.6-fixture"),
        ParamValue::Text(String::from("not-an-object")),
    );
    request
}

fn leaf_route_error(request: &BedrockPlanRequest) -> RouteError {
    let selector = ModelRouteSelector::new(
        &request.price_map,
        &request.converse_models,
        request.request_metadata_owned,
    )
    .expect("the fixture catalog compiles its route patterns");
    selector
        .route_for_request(
            &request.model,
            &request.request_params,
            Some(&request.additional_drop_params),
        )
        .expect_err("the malformed price entry refuses the route")
}

pub(super) fn invoke_plan() {
    let mut explicit = plain(CLAUDE);
    explicit.aws_region_name = Some(String::from("us-east-1"));
    check(
        explicit,
        &Expected {
            operation: BedrockRoute::Invoke,
            model_id: ENCODED_CLAUDE,
            region: "us-east-1",
            endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
        },
    );

    let mut prefixed = plain("bedrock/anthropic.claude-3-haiku-20240307-v1:0");
    prefixed.settings.aws_region_name = Some(String::from("eu-west-1"));
    check(
        prefixed,
        &Expected {
            operation: BedrockRoute::Invoke,
            model_id: ENCODED_CLAUDE,
            region: "eu-west-1",
            endpoint_url: "https://bedrock-runtime.eu-west-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.eu-west-1.amazonaws.com",
        },
    );

    let mut arn = plain(ARN);
    arn.api_base = Some(String::from("https://bedrock-proxy.example"));
    check(
        arn,
        &Expected {
            operation: BedrockRoute::Invoke,
            model_id: ENCODED_ARN,
            region: "us-west-2",
            endpoint_url: "https://bedrock-proxy.example",
            proxy_endpoint_url: "https://bedrock-proxy.example",
        },
    );

    let mut overrides = plain(CLAUDE);
    overrides.aws_region_name = Some(String::from("ap-south-1"));
    overrides.api_base = Some(String::from("https://bedrock-proxy.example"));
    overrides.aws_bedrock_runtime_endpoint = Some(String::from("https://bedrock-runtime.example"));
    check(
        overrides,
        &Expected {
            operation: BedrockRoute::Invoke,
            model_id: ENCODED_CLAUDE,
            region: "ap-south-1",
            endpoint_url: "https://bedrock-proxy.example",
            proxy_endpoint_url: "https://bedrock-runtime.example",
        },
    );

    let mut environment = plain("meta.llama3-70b-instruct-v1:0");
    environment.settings.session_region = Some(String::from("ca-central-1"));
    environment.settings.aws_bedrock_runtime_endpoint =
        Some(String::from("https://env-runtime.example"));
    check(
        environment,
        &Expected {
            operation: BedrockRoute::Invoke,
            model_id: "meta.llama3-70b-instruct-v1:0",
            region: "ca-central-1",
            endpoint_url: "https://env-runtime.example",
            proxy_endpoint_url: "https://env-runtime.example",
        },
    );

    let mut fallback = plain(CLAUDE);
    fallback.settings.aws_region = Some(String::new());
    let error =
        dispatch(&BedrockOperation::Plan(fallback)).expect_err("an empty region is invalid");
    assert!(matches!(
        error,
        BedrockDispatchError::Endpoint(AwsEndpointError::Region { .. })
    ));

    let mut defaulted = plain(CLAUDE);
    defaulted.aws_region_name = Some(String::from("us-west-2"));
    let rendered = dispatch(&BedrockOperation::Plan(defaulted))
        .expect("the plan composes the route and endpoint leaves");
    assert_eq!(rendered.region, "us-west-2");
    assert_eq!(rendered.endpoint_url, DEFAULT_ENDPOINT);

    assert_eq!(
        invoke_provider(CLAUDE),
        Some(BedrockInvokeProvider::Anthropic)
    );
    let leaf_region = llm_region_name(LlmRegion {
        explicit: Some("us-east-1"),
        model: Some(CLAUDE),
        model_id: None,
        environment_region_name: None,
        environment_region: None,
        session_region: None,
    })
    .expect("the fixture region is valid");
    let leaf_endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: None,
        configured_endpoint: None,
        environment_endpoint: None,
        region: &leaf_region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    assert_eq!(
        leaf_endpoint.endpoint_url,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(leaf_endpoint.proxy_endpoint_url, leaf_endpoint.endpoint_url);
}

pub(super) fn converse_plan() {
    let mut catalog_model = plain(CLAUDE);
    catalog_model.converse_models.insert(String::from(CLAUDE));
    catalog_model.aws_region_name = Some(String::from("us-east-1"));
    check(
        catalog_model.clone(),
        &Expected {
            operation: BedrockRoute::Converse,
            model_id: ENCODED_CLAUDE,
            region: "us-east-1",
            endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
        },
    );

    let mut prefixed = plain("converse/amazon.titan-text-express-v1");
    prefixed.aws_region_name = Some(String::from("eu-central-1"));
    check(
        prefixed,
        &Expected {
            operation: BedrockRoute::Converse,
            model_id: "amazon.titan-text-express-v1",
            region: "eu-central-1",
            endpoint_url: "https://bedrock-runtime.eu-central-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.eu-central-1.amazonaws.com",
        },
    );

    let mut overridden = plain(CLAUDE);
    overridden.converse_models.insert(String::from(CLAUDE));
    overridden.model_id = Some(String::from(OVERRIDE_ARN));
    check(
        overridden,
        &Expected {
            operation: BedrockRoute::Converse,
            model_id: ENCODED_OVERRIDE_ARN,
            region: "us-east-2",
            endpoint_url: "https://bedrock-runtime.us-east-2.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.us-east-2.amazonaws.com",
        },
    );

    let mut invoke_override = plain(CLAUDE);
    invoke_override.model_id = Some(String::from(OVERRIDE_ARN));
    check(
        invoke_override,
        &Expected {
            operation: BedrockRoute::Invoke,
            model_id: ENCODED_OVERRIDE_ARN,
            region: "us-west-2",
            endpoint_url: DEFAULT_ENDPOINT,
            proxy_endpoint_url: DEFAULT_ENDPOINT,
        },
    );

    let expected_route = ModelRouteSelector::new(
        &catalog_model.price_map,
        &catalog_model.converse_models,
        catalog_model.request_metadata_owned,
    )
    .expect("the fixture catalog compiles its route patterns")
    .route_for_request(
        &catalog_model.model,
        &catalog_model.request_params,
        Some(&catalog_model.additional_drop_params),
    )
    .expect("the catalog lists the model for converse");
    assert_eq!(expected_route, BedrockRoute::Converse);

    let malformed = malformed_catalog_request();
    let error = dispatch(&BedrockOperation::Plan(malformed.clone()))
        .expect_err("the malformed price entry refuses the plan");
    assert_eq!(error.to_string(), leaf_route_error(&malformed).to_string());
    assert!(matches!(
        error,
        BedrockDispatchError::Route(RouteError::MalformedPriceEntry { .. })
    ));
}

pub(super) fn operation_error_propagation() {
    let mut explicit = plain(CLAUDE);
    explicit.aws_region_name = Some(String::from("US_EAST_1"));
    let error = dispatch(&BedrockOperation::Plan(explicit)).expect_err("an invalid region refuses");
    let leaf_error = llm_region_name(LlmRegion {
        explicit: Some("US_EAST_1"),
        model: Some(CLAUDE),
        model_id: None,
        environment_region_name: None,
        environment_region: None,
        session_region: None,
    })
    .expect_err("the region leaf refuses uppercase letters");
    assert_eq!(error.to_string(), leaf_error.to_string());
    match error {
        BedrockDispatchError::Endpoint(AwsEndpointError::Region { region }) => {
            assert_eq!(region, "US_EAST_1");
        }
        other => panic!("the invalid region error was rewritten: {other}"),
    }

    let mut environment = plain(CLAUDE);
    environment.settings.aws_region = Some(String::new());
    let error =
        dispatch(&BedrockOperation::Plan(environment)).expect_err("an empty region refuses");
    let leaf_error = llm_region_name(LlmRegion {
        explicit: None,
        model: Some(CLAUDE),
        model_id: None,
        environment_region_name: None,
        environment_region: Some(""),
        session_region: None,
    })
    .expect_err("the region leaf refuses an empty region");
    assert_eq!(error.to_string(), leaf_error.to_string());
    match error {
        BedrockDispatchError::Endpoint(AwsEndpointError::Region { region }) => {
            assert!(region.is_empty());
        }
        other => panic!("the empty region error was rewritten: {other}"),
    }

    let malformed = malformed_catalog_request();
    let error = dispatch(&BedrockOperation::Plan(malformed.clone()))
        .expect_err("the malformed price entry refuses the plan");
    assert_eq!(error.to_string(), leaf_route_error(&malformed).to_string());
    match error {
        BedrockDispatchError::Route(RouteError::MalformedPriceEntry { key }) => {
            assert_eq!(key, "openai.gpt-5.6-fixture");
        }
        other => panic!("the catalog error was rewritten: {other}"),
    }
}
