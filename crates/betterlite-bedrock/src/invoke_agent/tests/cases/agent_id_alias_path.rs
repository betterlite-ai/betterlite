use betterlite_http::error::HttpError;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::invoke_agent::error::InvokeAgentRequestError;
use crate::invoke_agent::identifiers::agent_id_and_alias_id;
use crate::invoke_agent::tests::support::{SESSION_UUID, generated_uuid, params};
use crate::invoke_agent::url::{CompleteUrl, InvokeEnvironment, complete_url};
use crate::model_routes::ParamMap;
use crate::signing::target::RegionInputs;

const MODEL: &str = "agent/L1RT58GYRW/MFPSBCXYTW";
const DEFAULT_ENDPOINT: &str = "https://bedrock-agent-runtime.us-west-2.amazonaws.com";

pub(super) fn agent_id_alias_path() {
    assert_model_shape();
    assert_encoded_segments();
    assert_endpoint_precedence();
    assert_region_precedence();
}

fn environment(
    runtime_endpoint: Option<&str>,
    region_name: Option<&str>,
    region: Option<&str>,
    session_region: Option<&str>,
) -> InvokeEnvironment<'_> {
    InvokeEnvironment {
        runtime_endpoint,
        region_inputs: RegionInputs {
            environment_region_name: region_name,
            environment_region: region,
            session_region,
        },
    }
}

fn try_url(
    api_base: Option<&str>,
    optional_params: &ParamMap,
    env: InvokeEnvironment<'_>,
) -> Result<String, InvokeAgentRequestError> {
    complete_url(CompleteUrl {
        api_base,
        model: MODEL,
        optional_params,
        environment: env,
        generated_session_id: generated_uuid(),
    })
}

fn url(optional_params: &ParamMap) -> Result<String, InvokeAgentRequestError> {
    try_url(None, optional_params, InvokeEnvironment::default())
}

fn path(session: &str) -> String {
    format!("/agents/L1RT58GYRW/agentAliases/MFPSBCXYTW/sessions/{session}/text")
}

fn generated_path() -> String {
    path(SESSION_UUID)
}

fn assert_model_shape() {
    assert_eq!(agent_id_and_alias_id("agent/A/B").unwrap(), ("A", "B"));
    for model in [
        "agent/A",
        "agent/A/B/C",
        "notagent/A/B",
        "Agent/A/B",
        "agent",
        "",
    ] {
        assert!(
            matches!(
                agent_id_and_alias_id(model),
                Err(InvokeAgentRequestError::InvalidModelFormat)
            ),
            "{model}"
        );
    }
    assert_eq!(
        InvokeAgentRequestError::InvalidModelFormat.to_string(),
        "Invalid model format. Expected format: 'model=agent/AGENT_ID/ALIAS_ID'"
    );
    assert_eq!(
        agent_id_and_alias_id("agent//ALIAS").unwrap(),
        ("", "ALIAS")
    );
    assert_eq!(agent_id_and_alias_id("agent/ID/").unwrap(), ("ID", ""));
    assert!(matches!(
        url_with_model("agent//ALIAS", &params("{}")),
        Err(InvokeAgentRequestError::Segment(HttpError::Required { field_name }))
            if field_name == "agent_id"
    ));
    assert!(matches!(
        url_with_model("agent/ID/", &params("{}")),
        Err(InvokeAgentRequestError::Segment(HttpError::Required { field_name }))
            if field_name == "agent_alias_id"
    ));
    assert!(matches!(
        url_with_model("agent/../ALIAS", &params("{}")),
        Err(InvokeAgentRequestError::Segment(HttpError::DotSegment { field_name }))
            if field_name == "agent_id"
    ));
    assert!(matches!(
        url_with_model("agent/ID/..", &params("{}")),
        Err(InvokeAgentRequestError::Segment(HttpError::DotSegment { field_name }))
            if field_name == "agent_alias_id"
    ));
}

fn assert_encoded_segments() {
    assert_eq!(
        url(&params("{}")).unwrap(),
        format!("{DEFAULT_ENDPOINT}{}", generated_path())
    );
    let punctuation = "agent/id with +&=?#% \u{e9}/alias";
    assert_eq!(
        url_with_model(punctuation, &params(r#"{"sessionID":"s 1"}"#)).unwrap(),
        format!(
            "{DEFAULT_ENDPOINT}/agents/id%20with%20%2B%26%3D%3F%23%25%20%C3%A9/agentAliases/alias/sessions/s%201/text"
        )
    );
}

fn url_with_model(
    model: &str,
    optional_params: &ParamMap,
) -> Result<String, InvokeAgentRequestError> {
    complete_url(CompleteUrl {
        api_base: None,
        model,
        optional_params,
        environment: InvokeEnvironment::default(),
        generated_session_id: generated_uuid(),
    })
}

fn assert_endpoint_precedence() {
    let configured = params(r#"{"aws_bedrock_runtime_endpoint":"https://vpce.example/prefix/"}"#);
    assert_eq!(
        try_url(
            Some("https://proxy.example/base"),
            &configured,
            environment(Some("https://env.example"), None, None, None)
        )
        .unwrap(),
        format!("https://proxy.example/base{}", generated_path()),
        "api_base takes the source endpoint precedence"
    );
    assert_eq!(
        try_url(
            None,
            &configured,
            environment(Some("https://env.example"), None, None, None)
        )
        .unwrap(),
        format!("https://vpce.example/prefix/{}", generated_path()),
        "the configured endpoint wins over the environment and keeps its trailing slash"
    );
    assert_eq!(
        try_url(
            None,
            &params("{}"),
            environment(Some("https://env.example"), None, None, None)
        )
        .unwrap(),
        format!("https://env.example{}", generated_path())
    );
    assert_eq!(
        try_url(None, &params("{}"), environment(Some(""), None, None, None)).unwrap(),
        format!("{DEFAULT_ENDPOINT}{}", generated_path()),
        "an empty environment endpoint counts as unset"
    );
    assert_eq!(
        try_url(
            None,
            &params(r#"{"aws_bedrock_runtime_endpoint":""}"#),
            InvokeEnvironment::default()
        )
        .unwrap(),
        generated_path(),
        "an explicit empty endpoint is used as given"
    );
    assert_eq!(
        url(&params(r#"{"aws_bedrock_runtime_endpoint":null}"#)).unwrap(),
        format!("{DEFAULT_ENDPOINT}{}", generated_path())
    );
    assert!(matches!(
        url(&params(r#"{"aws_bedrock_runtime_endpoint":7}"#)),
        Err(InvokeAgentRequestError::WrongType {
            field: "aws_bedrock_runtime_endpoint"
        })
    ));
}

fn assert_region_precedence() {
    assert_eq!(
        url(&params(r#"{"aws_region_name":"eu-west-1"}"#)).unwrap(),
        format!(
            "https://bedrock-agent-runtime.eu-west-1.amazonaws.com{}",
            generated_path()
        )
    );
    assert_eq!(
        try_url(
            None,
            &params("{}"),
            environment(None, Some("ap-south-1"), None, None)
        )
        .unwrap(),
        format!(
            "https://bedrock-agent-runtime.ap-south-1.amazonaws.com{}",
            generated_path()
        ),
        "AWS_REGION_NAME is read when no region is explicit"
    );
    assert_eq!(
        try_url(
            None,
            &params("{}"),
            environment(None, None, Some("ca-central-1"), None)
        )
        .unwrap(),
        format!(
            "https://bedrock-agent-runtime.ca-central-1.amazonaws.com{}",
            generated_path()
        ),
        "AWS_REGION is read after AWS_REGION_NAME"
    );
    assert_eq!(
        try_url(
            None,
            &params("{}"),
            environment(None, Some("ap-south-1"), Some("ca-central-1"), None)
        )
        .unwrap(),
        format!(
            "https://bedrock-agent-runtime.ap-south-1.amazonaws.com{}",
            generated_path()
        )
    );
    assert_eq!(
        try_url(
            None,
            &params("{}"),
            environment(None, None, None, Some("sa-east-1"))
        )
        .unwrap(),
        format!(
            "https://bedrock-agent-runtime.sa-east-1.amazonaws.com{}",
            generated_path()
        ),
        "the session region is read last"
    );
    assert_eq!(
        try_url(None, &params("{}"), environment(None, None, None, Some(""))).unwrap(),
        format!("{DEFAULT_ENDPOINT}{}", generated_path())
    );
    assert_eq!(
        try_url(
            None,
            &params(r#"{"aws_region_name":"eu-west-1"}"#),
            environment(None, Some("ap-south-1"), None, None)
        )
        .unwrap(),
        format!(
            "https://bedrock-agent-runtime.eu-west-1.amazonaws.com{}",
            generated_path()
        ),
        "the explicit region wins over the environment"
    );
    assert_eq!(
        try_url(
            None,
            &params("{}"),
            environment(None, None, None, Some("cn-north-1"))
        )
        .unwrap(),
        format!(
            "https://bedrock-agent-runtime.cn-north-1.amazonaws.com.cn{}",
            generated_path()
        )
    );
    assert_eq!(
        url(&params(r#"{"aws_region_name":null}"#)).unwrap(),
        format!("{DEFAULT_ENDPOINT}{}", generated_path())
    );
    assert!(matches!(
        url(&params(r#"{"aws_region_name":"US_EAST"}"#)),
        Err(InvokeAgentRequestError::Endpoint(
            AwsEndpointError::Region { region }
        )) if region == "US_EAST"
    ));
    assert!(matches!(
        try_url(
            None,
            &params("{}"),
            environment(None, Some("US_EAST"), None, None)
        ),
        Err(InvokeAgentRequestError::Endpoint(
            AwsEndpointError::Region { region }
        )) if region == "US_EAST"
    ));
    assert!(matches!(
        url(&params(r#"{"aws_region_name":7}"#)),
        Err(InvokeAgentRequestError::WrongType {
            field: "aws_region_name"
        })
    ));
}
