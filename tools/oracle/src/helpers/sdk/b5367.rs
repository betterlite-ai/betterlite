use std::sync::Arc;
use std::time::Duration;

use betterlite_core::rules::RuleError;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_state::correlation::CorrelationSettings;
use betterlite_logging::logging_state::types::DynamicCallbacks;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::callbacks::CallbackLists;
use betterlite_sdk::call_setup::callbacks::CustomLoggers;
use betterlite_sdk::call_setup::credentials::Credential;
use betterlite_sdk::call_setup::credentials::CredentialLookup;
use betterlite_sdk::call_setup::credentials::load_credentials_from_list;
use betterlite_sdk::call_setup::function_setup::CallSetup;
use betterlite_sdk::call_setup::function_setup::CallSetupDependencies;
use betterlite_sdk::call_setup::function_setup::CallSetupError;
use betterlite_sdk::call_setup::function_setup::TraceSource;
use betterlite_sdk::call_setup::function_setup::function_setup;
use betterlite_sdk::call_setup::hooks::DeploymentHook;
use betterlite_sdk::call_setup::providers::ProviderRegistry;
use betterlite_sdk::call_setup::request_flags::is_async_request;
use betterlite_sdk::call_setup::request_flags::is_streaming_request;
use serde_json::Value;
use serde_json::json;

use self::input::Action;
use self::input::Input;
use self::input::moment;
use self::input::redactor;
use self::input::request_map;
use self::support::CredentialSpec;
use self::support::Markers;
use self::support::Resolver;
use self::support::Rules;
use self::support::global_callback;
use self::support::helper_error;
use self::support::lists_value;

mod input;
mod support;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.action {
            Action::Setup => setup(input),
            Action::Credentials => credentials(input),
            Action::RequestFlags => request_flags(input),
        }
    }
}

struct Session {
    markers: Markers,
    callbacks: Vec<Arc<dyn DeploymentHook>>,
    providers: ProviderRegistry,
    correlation: CorrelationContext,
    redactor: Redactor,
    lists: CallbackLists,
}

impl Session {
    fn new(input: &Input) -> Result<Self, Box<crate::helpers::HelperError>> {
        let markers = Markers::default();
        let callbacks: Vec<Arc<dyn DeploymentHook>> = input
            .callbacks
            .iter()
            .map(|spec| global_callback(spec, &markers))
            .collect();
        let lists = input.lists.build(&markers);
        Ok(Self {
            markers,
            callbacks,
            providers: ProviderRegistry {
                custom_provider_map: input.provider_map.clone(),
                providers: input.providers.clone(),
                custom_providers: input.custom_providers.clone(),
            },
            correlation: CorrelationContext::default(),
            redactor: redactor()?,
            lists,
        })
    }
}

fn setup(input: &Input) -> crate::helpers::HelperResult {
    let mut session = Session::new(input)?;
    let call = invoke(input, &mut session)?;
    setup_value(&call, &session.providers, &session.lists)
}

fn invoke(
    input: &Input,
    session: &mut Session,
) -> Result<CallSetup, Box<crate::helpers::HelperError>> {
    let resolver = Resolver {
        outcomes: &input.resolver,
        markers: &session.markers,
    };
    let known = |name: &str| input.resolver.contains_key(name);
    let loggers = CustomLoggers {
        known: &known,
        resolver: &resolver,
    };
    let mut ticks = 0_u64;
    let mut clock = || {
        ticks = ticks.saturating_add(1);
        Duration::from_millis(ticks)
    };
    let trace_id = String::from("generated-trace");
    let next_trace_id = || trace_id.clone();
    let deps = CallSetupDependencies {
        lists: &mut session.lists,
        providers: &mut session.providers,
        callbacks: &session.callbacks,
        loggers,
        correlation: &mut session.correlation,
        correlation_settings: CorrelationSettings::default(),
        redactor: &session.redactor,
        trace: TraceSource {
            next_trace_id: &next_trace_id,
            clock: &mut clock,
        },
        breadcrumbs: None,
        litellm_params: RequestMap::default(),
        turn_off_message_logging: false,
        is_async_call: input.is_async_call.unwrap_or(true),
    };
    let evaluator = Rules {
        rules: &input.rules,
    };
    let request = request_map(&input.request)?;
    function_setup(&input.call_type, &evaluator, moment(), request, deps)
        .map_err(|error| setup_failure(&error))
}

fn setup_failure(error: &CallSetupError) -> Box<crate::helpers::HelperError> {
    let status = match error {
        CallSetupError::Rule(RuleError::Rejected { .. }) => Some(500),
        CallSetupError::Rule(RuleError::MissingInput)
        | CallSetupError::Logging(_)
        | CallSetupError::Guardrail(_)
        | CallSetupError::Callback(_)
        | CallSetupError::FieldType { .. }
        | CallSetupError::MissingField { .. }
        | CallSetupError::Json { .. } => None,
    };
    let mut failure = helper_error("SetupError", error.to_string());
    failure.status = status;
    failure
}

fn setup_value(
    call: &CallSetup,
    providers: &ProviderRegistry,
    lists: &CallbackLists,
) -> crate::helpers::HelperResult {
    Ok(json!({
        "request": serde_json::to_value(&call.request)
            .map_err(|error| helper_error("InvalidInput", error.to_string()))?,
        "lists": lists_value(lists),
        "providers": {
            "providers": providers.providers,
            "custom_providers": providers.custom_providers,
        },
        "logging": logging_value(call),
    }))
}

fn logging_value(call: &CallSetup) -> Value {
    let dynamic = dynamic_value(&call.logging.dynamic_callbacks);
    json!({
        "model": call.logging.model,
        "call_type": call.logging.call_type,
        "function_id": call.logging.function_id,
        "stream": call.logging.stream,
        "applied_guardrails": call.logging.applied_guardrails,
        "dynamic": dynamic,
    })
}

fn dynamic_value(dynamic: &DynamicCallbacks) -> Value {
    json!({
        "input": dynamic.input,
        "success": dynamic.success,
        "async_success": dynamic.async_success,
        "failure": dynamic.failure,
        "async_failure": dynamic.async_failure,
    })
}

fn credentials(input: &Input) -> crate::helpers::HelperResult {
    let mut request = request_map(&input.request)?;
    let lookup = CredentialList {
        entries: input
            .credentials
            .iter()
            .map(CredentialSpec::credential)
            .collect(),
    };
    let warning = load_credentials_from_list(&mut request, &lookup)
        .map_err(|error| helper_error("CredentialError", error.to_string()))?;
    Ok(json!({
        "request": serde_json::to_value(&request)
            .map_err(|error| helper_error("InvalidInput", error.to_string()))?,
        "warning": warning.map(|found| found.credential_name),
    }))
}

fn request_flags(input: &Input) -> crate::helpers::HelperResult {
    let request = request_map(&input.request)?;
    Ok(json!({
        "async": is_async_request(&request, input.is_pass_through.unwrap_or(false)),
        "streaming": is_streaming_request(&request, &input.call_type),
    }))
}

struct CredentialList {
    entries: Vec<Credential>,
}

impl CredentialLookup for CredentialList {
    fn find_credential(&self, credential_name: &str) -> Option<&Credential> {
        self.entries
            .iter()
            .find(|credential| credential.credential_name == credential_name)
    }
}
