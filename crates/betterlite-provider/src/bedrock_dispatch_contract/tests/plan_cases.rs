use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::region::{LlmRegion, llm_region_name};
use betterlite_bedrock::model_routes::{
    BedrockRoute, ModelRouteSelector, ParamMap, ParamValue, RouteError,
};
use indexmap::{IndexMap, IndexSet};

use crate::bedrock_dispatch_contract::{
    BedrockAwsSettings, BedrockDispatchError, BedrockOperation, BedrockPlanRequest,
    BedrockPrepared, dispatch,
};

const CLAUDE: &str = "anthropic.claude-3-haiku-20240307-v1:0";
const ARN: &str =
    "arn:aws:bedrock:eu-west-3:123456789012:inference-profile/global.anthropic.claude";
const ENCODED_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aeu-west-3%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude";
const OVERRIDE_ARN: &str =
    "arn:aws:bedrock:us-east-2:123456789012:inference-profile/global.anthropic.claude";
const ENCODED_OVERRIDE_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aus-east-2%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude";
const MALFORMED_MODEL: &str = "openai.gpt-5.6-broken";
const GPT: &str = "openai.gpt-5.6-fixture";
const PROXY: &str = "https://bedrock-proxy.example";

#[derive(Clone, Copy)]
struct Expected {
    model_id: &'static str,
    region: &'static str,
    endpoint_url: &'static str,
    proxy_endpoint_url: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct Row {
    model: &'static str,
    model_id: Option<&'static str>,
    region: Option<&'static str>,
    api_base: Option<&'static str>,
    runtime_endpoint: Option<&'static str>,
    settings_region_name: Option<&'static str>,
    settings_region: Option<&'static str>,
    session_region: Option<&'static str>,
    settings_endpoint: Option<&'static str>,
    converse: bool,
    expected: Expected,
}

pub(super) const BASE: Row = Row {
    model: CLAUDE,
    model_id: None,
    region: None,
    api_base: None,
    runtime_endpoint: None,
    settings_region_name: None,
    settings_region: None,
    session_region: None,
    settings_endpoint: None,
    converse: false,
    expected: Expected {
        model_id: CLAUDE,
        region: "us-west-2",
        endpoint_url: "https://bedrock-runtime.us-west-2.amazonaws.com",
        proxy_endpoint_url: "https://bedrock-runtime.us-west-2.amazonaws.com",
    },
};

const US_EAST_1: Expected = Expected {
    region: "us-east-1",
    endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
    proxy_endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
    ..BASE.expected
};

pub(super) const INVOKE_ROWS: [Row; 8] = [
    Row {
        region: Some("us-east-1"),
        expected: US_EAST_1,
        ..BASE
    },
    BASE,
    Row {
        model: "bedrock/anthropic.claude-3-haiku-20240307-v1:0",
        settings_region_name: Some("eu-west-1"),
        settings_region: Some("sa-east-1"),
        expected: Expected {
            region: "eu-west-1",
            endpoint_url: "https://bedrock-runtime.eu-west-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.eu-west-1.amazonaws.com",
            ..BASE.expected
        },
        ..BASE
    },
    Row {
        settings_region: Some("sa-east-1"),
        session_region: Some("ca-central-1"),
        expected: Expected {
            region: "sa-east-1",
            endpoint_url: "https://bedrock-runtime.sa-east-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.sa-east-1.amazonaws.com",
            ..BASE.expected
        },
        ..BASE
    },
    Row {
        model: ARN,
        api_base: Some(PROXY),
        expected: Expected {
            model_id: ENCODED_ARN,
            region: "eu-west-3",
            endpoint_url: PROXY,
            proxy_endpoint_url: PROXY,
        },
        ..BASE
    },
    Row {
        region: Some("ap-south-1"),
        api_base: Some(PROXY),
        runtime_endpoint: Some("https://bedrock-runtime.example"),
        settings_endpoint: Some("https://env-runtime.example"),
        expected: Expected {
            region: "ap-south-1",
            endpoint_url: PROXY,
            proxy_endpoint_url: "https://bedrock-runtime.example",
            ..BASE.expected
        },
        ..BASE
    },
    Row {
        model: "meta.llama3-70b-instruct-v1:0",
        session_region: Some("ca-central-1"),
        settings_endpoint: Some("https://env-runtime.example"),
        expected: Expected {
            model_id: "meta.llama3-70b-instruct-v1:0",
            region: "ca-central-1",
            endpoint_url: "https://env-runtime.example",
            proxy_endpoint_url: "https://env-runtime.example",
        },
        ..BASE
    },
    // Invoke resolves the region from the model alone, so the override's us-east-2 never wins
    Row {
        model_id: Some(OVERRIDE_ARN),
        settings_region_name: Some("eu-north-1"),
        expected: Expected {
            model_id: ENCODED_OVERRIDE_ARN,
            region: "eu-north-1",
            endpoint_url: "https://bedrock-runtime.eu-north-1.amazonaws.com",
            proxy_endpoint_url: "https://bedrock-runtime.eu-north-1.amazonaws.com",
        },
        ..BASE
    },
];

const CONVERSE_ROWS: [Row; 5] = [
    Row {
        converse: true,
        ..BASE
    },
    Row {
        model: "converse/amazon.titan-text-express-v1",
        region: Some("eu-central-1"),
        ..BASE
    },
    Row {
        model: "converse/us-east-1/anthropic.claude-3-haiku-20240307-v1:0",
        ..BASE
    },
    // the Invoke region check never runs on a routed model, so an invalid region still plans
    Row {
        model: "converse/amazon.titan-text-express-v1",
        region: Some("US_EAST_1"),
        ..BASE
    },
    Row {
        model_id: Some(OVERRIDE_ARN),
        converse: true,
        ..BASE
    },
];

fn request(row: &Row) -> BedrockPlanRequest {
    let mut converse_models = IndexSet::new();
    if row.converse {
        converse_models.insert(row.model.to_owned());
    }
    BedrockPlanRequest {
        model: row.model.to_owned(),
        model_id: row.model_id.map(String::from),
        aws_region_name: row.region.map(String::from),
        api_base: row.api_base.map(String::from),
        aws_bedrock_runtime_endpoint: row.runtime_endpoint.map(String::from),
        settings: BedrockAwsSettings {
            aws_region_name: row.settings_region_name.map(String::from),
            aws_region: row.settings_region.map(String::from),
            session_region: row.session_region.map(String::from),
            aws_bedrock_runtime_endpoint: row.settings_endpoint.map(String::from),
        },
        request_params: ParamMap::new(),
        additional_drop_params: Vec::new(),
        price_map: ParamMap::new(),
        converse_models,
        request_metadata_owned: false,
    }
}

pub(super) fn check_rows(rows: &[Row]) {
    for row in rows {
        let plan = dispatch(&BedrockOperation::Plan(Box::new(request(row))))
            .expect("the plan composes the route and endpoint leaves");
        let expected = BedrockPrepared::Invoke {
            model: row.model.to_owned(),
            model_id: row.expected.model_id.to_owned(),
            region: row.expected.region.to_owned(),
            endpoint_url: row.expected.endpoint_url.to_owned(),
            proxy_endpoint_url: row.expected.proxy_endpoint_url.to_owned(),
        };
        assert_eq!(plan, expected, "{}", row.model);
    }
}

fn check_routed(request: BedrockPlanRequest, operation: BedrockRoute) {
    let model = request.model.clone();
    let plan = dispatch(&BedrockOperation::Plan(Box::new(request)))
        .expect("a route other than Invoke still plans");
    assert_eq!(plan, BedrockPrepared::Routed { operation, model });
}

// B598 picks chat_completions for a GPT 5.6 row listing the endpoint, Converse once stop is sent, back when stop is dropped, and Converse again when request metadata is owned
fn check_params_reach_route() {
    let endpoints = ParamValue::List(vec![ParamValue::Text(String::from("/v1/chat/completions"))]);
    let entry = IndexMap::from([(String::from("supported_endpoints"), endpoints)]);
    let mut gpt = request(&Row { model: GPT, ..BASE });
    gpt.price_map
        .insert(GPT.to_owned(), ParamValue::Mapping(entry));
    check_routed(gpt.clone(), BedrockRoute::ChatCompletions);
    let stop = ParamValue::List(vec![ParamValue::Text(String::from("x"))]);
    gpt.request_params.insert(String::from("stop"), stop);
    check_routed(gpt.clone(), BedrockRoute::Converse);
    gpt.additional_drop_params.push(String::from("stop"));
    check_routed(gpt.clone(), BedrockRoute::ChatCompletions);
    gpt.request_metadata_owned = true;
    check_routed(gpt, BedrockRoute::Converse);
}

// a price entry that is not an object makes B598's route selector refuse, and the plan must keep that error as is
fn assert_catalog_error() {
    let mut malformed = request(&Row {
        model: MALFORMED_MODEL,
        region: Some("us-east-1"),
        ..BASE
    });
    malformed.price_map.insert(
        MALFORMED_MODEL.to_owned(),
        ParamValue::Text(String::from("not-an-object")),
    );
    let leaf_error =
        ModelRouteSelector::new(&malformed.price_map, &malformed.converse_models, false)
            .expect("the fixture catalog compiles its route patterns")
            .route_for_request(MALFORMED_MODEL, &malformed.request_params, Some(&[]))
            .expect_err("the malformed price entry refuses the route");
    let error = dispatch(&BedrockOperation::Plan(Box::new(malformed)))
        .expect_err("the malformed price entry refuses the plan");
    assert_eq!(error.to_string(), leaf_error.to_string());
    match error {
        BedrockDispatchError::Route(RouteError::MalformedPriceEntry { key }) => {
            assert_eq!(key, MALFORMED_MODEL);
        }
        BedrockDispatchError::Route(_)
        | BedrockDispatchError::Endpoint(_)
        | BedrockDispatchError::Encode { .. }
        | BedrockDispatchError::Unsupported { .. } => panic!("the catalog error was rewritten"),
    }
}

fn assert_region_error(row: &Row, leaf: LlmRegion<'_>, region: &str) {
    let error = dispatch(&BedrockOperation::Plan(Box::new(request(row))))
        .expect_err("an invalid region refuses the plan");
    let leaf_error = llm_region_name(leaf).expect_err("the region leaf refuses the fixture");
    assert_eq!(error.to_string(), leaf_error.to_string());
    match error {
        BedrockDispatchError::Endpoint(AwsEndpointError::Region { region: found }) => {
            assert_eq!(found, region);
        }
        BedrockDispatchError::Route(_)
        | BedrockDispatchError::Endpoint(_)
        | BedrockDispatchError::Encode { .. }
        | BedrockDispatchError::Unsupported { .. } => panic!("the region error was rewritten"),
    }
}

pub(super) fn converse_plan() {
    for row in &CONVERSE_ROWS {
        check_routed(request(row), BedrockRoute::Converse);
    }
    check_params_reach_route();
    assert_catalog_error();
}

pub(super) fn operation_error_propagation() {
    let leaf = LlmRegion {
        explicit: None,
        model: Some(CLAUDE),
        model_id: None,
        environment_region_name: None,
        environment_region: None,
        session_region: None,
    };
    assert_region_error(
        &Row {
            region: Some("US_EAST_1"),
            ..BASE
        },
        LlmRegion {
            explicit: Some("US_EAST_1"),
            ..leaf
        },
        "US_EAST_1",
    );
    assert_region_error(
        &Row {
            settings_region: Some(""),
            ..BASE
        },
        LlmRegion {
            environment_region: Some(""),
            ..leaf
        },
        "",
    );
    assert_catalog_error();
}
