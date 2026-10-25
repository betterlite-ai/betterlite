use betterlite_auth::auxiliary_routes;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    group: String,
}

pub(crate) struct Op;

fn route_values(group: &str) -> Result<Vec<&'static str>, String> {
    let routes = match group {
        "openai_route_names" => auxiliary_routes::OPENAI_ROUTE_NAMES.to_vec(),
        "mapped_pass_through_routes" => auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES.to_vec(),
        "passthrough_routes_wildcard" => auxiliary_routes::PASSTHROUGH_ROUTES_WILDCARD.to_vec(),
        "litellm_native_routes" => auxiliary_routes::LITELLM_NATIVE_ROUTES.to_vec(),
        "mcp_inference_routes" => auxiliary_routes::MCP_INFERENCE_ROUTES.to_vec(),
        "mcp_management_routes" => auxiliary_routes::MCP_MANAGEMENT_ROUTES.to_vec(),
        "mcp_routes" => auxiliary_routes::MCP_ROUTES.to_vec(),
        "agent_inference_routes" => auxiliary_routes::AGENT_INFERENCE_ROUTES.to_vec(),
        "agent_management_routes" => auxiliary_routes::AGENT_MANAGEMENT_ROUTES.to_vec(),
        "agent_routes" => auxiliary_routes::AGENT_ROUTES.to_vec(),
        "google_routes" => auxiliary_routes::GOOGLE_ROUTES.to_vec(),
        "apply_guardrail_routes" => auxiliary_routes::APPLY_GUARDRAIL_ROUTES.to_vec(),
        "model_info_routes" => auxiliary_routes::MODEL_INFO_ROUTES.to_vec(),
        "info_routes" => auxiliary_routes::INFO_ROUTES.to_vec(),
        "public_routes" => auxiliary_routes::public_routes().into_iter().collect(),
        "ui_routes" => auxiliary_routes::UI_ROUTES.to_vec(),
        "compliance_check_routes" => auxiliary_routes::COMPLIANCE_CHECK_ROUTES.to_vec(),
        "self_managed_routes" => auxiliary_routes::SELF_MANAGED_ROUTES.to_vec(),
        _ => return Err(format!("unknown auxiliary route group: {group}")),
    };
    Ok(routes)
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let routes = route_values(&input.group).map_err(|message| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        })?;
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
