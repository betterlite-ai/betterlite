use betterlite_auth::auxiliary_routes;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Group {
    OpenaiRouteNames,
    MappedPassThroughRoutes,
    PassthroughRoutesWildcard,
    LitellmNativeRoutes,
    McpInferenceRoutes,
    McpManagementRoutes,
    McpRoutes,
    AgentInferenceRoutes,
    AgentManagementRoutes,
    AgentRoutes,
    GoogleRoutes,
    ApplyGuardrailRoutes,
    ModelInfoRoutes,
    InfoRoutes,
    PublicRoutes,
    UiRoutes,
    ComplianceCheckRoutes,
    SelfManagedRoutes,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    group: Group,
}

pub(crate) struct Op;

fn route_values(group: &Group) -> Vec<&'static str> {
    match group {
        Group::OpenaiRouteNames => auxiliary_routes::OPENAI_ROUTE_NAMES.to_vec(),
        Group::MappedPassThroughRoutes => auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES.to_vec(),
        Group::PassthroughRoutesWildcard => auxiliary_routes::PASSTHROUGH_ROUTES_WILDCARD.to_vec(),
        Group::LitellmNativeRoutes => auxiliary_routes::LITELLM_NATIVE_ROUTES.to_vec(),
        Group::McpInferenceRoutes => auxiliary_routes::MCP_INFERENCE_ROUTES.to_vec(),
        Group::McpManagementRoutes => auxiliary_routes::MCP_MANAGEMENT_ROUTES.to_vec(),
        Group::McpRoutes => auxiliary_routes::MCP_ROUTES.to_vec(),
        Group::AgentInferenceRoutes => auxiliary_routes::AGENT_INFERENCE_ROUTES.to_vec(),
        Group::AgentManagementRoutes => auxiliary_routes::AGENT_MANAGEMENT_ROUTES.to_vec(),
        Group::AgentRoutes => auxiliary_routes::AGENT_ROUTES.to_vec(),
        Group::GoogleRoutes => auxiliary_routes::GOOGLE_ROUTES.to_vec(),
        Group::ApplyGuardrailRoutes => auxiliary_routes::APPLY_GUARDRAIL_ROUTES.to_vec(),
        Group::ModelInfoRoutes => auxiliary_routes::MODEL_INFO_ROUTES.to_vec(),
        Group::InfoRoutes => auxiliary_routes::INFO_ROUTES.to_vec(),
        Group::PublicRoutes => auxiliary_routes::public_routes().into_iter().collect(),
        Group::UiRoutes => auxiliary_routes::UI_ROUTES.to_vec(),
        Group::ComplianceCheckRoutes => auxiliary_routes::COMPLIANCE_CHECK_ROUTES.to_vec(),
        Group::SelfManagedRoutes => auxiliary_routes::SELF_MANAGED_ROUTES.to_vec(),
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let routes = route_values(&input.group);
        serde_json::to_value(routes).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
