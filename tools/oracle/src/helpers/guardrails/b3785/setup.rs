use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::javelin::AppliedGuardrailsHeader;
use betterlite_guardrails::pangea::config::{PangeaGuardrailConfigModel, PangeaOptionalParams};
use betterlite_guardrails::pangea::hooks::{PangeaGuardrail, RequestData};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::HelperError;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptSpec, ScriptedReply, Wait};

const HANGING_RESPONSE_MILLIS: u64 = 5_000;
const HANGING_SCRIPT_DEADLINE_MILLIS: u64 = 30_000;
const HANGING_CLIENT_TIMEOUT: Duration = Duration::from_millis(100);

pub(super) struct Recorder;

impl AppliedGuardrailsHeader for Recorder {
    fn record_applied_guardrail(&mut self, _guardrail_name: &str) {}
}

pub(super) fn fail(class: &str, message: String, status: Option<u16>) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail("OSError", error.to_string(), None))
}

struct Endpoint<'call> {
    api_key: &'call str,
    guardrail_name: &'call str,
    input_recipe: Option<&'call String>,
    output_recipe: Option<&'call String>,
    base: &'call str,
    hanging: bool,
}

impl Endpoint<'_> {
    fn guardrail(&self) -> Result<PangeaGuardrail, Box<HelperError>> {
        let config = PangeaGuardrailConfigModel {
            api_key: Some(SecretString::from(self.api_key.to_owned())),
            api_base: Some(self.base.to_owned()),
            optional_params: Some(PangeaOptionalParams {
                pangea_input_recipe: self.input_recipe.cloned(),
                pangea_output_recipe: self.output_recipe.cloned(),
            }),
        };
        let builder = base_builder();
        let configured = if self.hanging {
            builder.timeout(HANGING_CLIENT_TIMEOUT)
        } else {
            builder
        };
        let client = configured
            .build()
            .map_err(|error| fail("RequestError", error.to_string(), None))?;
        PangeaGuardrail::new(
            Some(self.guardrail_name.to_owned()),
            config,
            &BTreeMap::new(),
            client,
        )
        .map_err(|error| fail("ValueError", error.to_string(), None))
    }
}

fn start_mock(
    answer: Option<&UpstreamAnswer>,
    hanging: bool,
) -> Result<(MockProvider, String), Box<HelperError>> {
    let script = if hanging {
        let reply = ScriptedReply {
            wait: Wait::Delay {
                millis: HANGING_RESPONSE_MILLIS,
            },
            then: Outcome::Answer(UpstreamAnswer {
                status: 200,
                headers: BTreeMap::new(),
                body: Some(json!({})),
                events: None,
            }),
        };
        MockScript::try_from(ScriptSpec {
            deadline_ms: HANGING_SCRIPT_DEADLINE_MILLIS,
            replies: vec![reply],
        })
    } else {
        let reply =
            answer.ok_or_else(|| fail("MockError", "the case has no answer".to_owned(), None))?;
        MockScript::try_from(std::slice::from_ref(reply))
    }
    .map_err(|error| fail("MockError", error.to_string(), None))?;
    let mock =
        MockProvider::start(script).map_err(|error| fail("MockError", error.to_string(), None))?;
    let base = mock.base_url.clone();
    Ok((mock, base))
}

pub(super) struct Common<'call> {
    pub(super) api_key: &'call str,
    pub(super) guardrail_name: &'call str,
    pub(super) input_recipe: Option<&'call String>,
    pub(super) output_recipe: Option<&'call String>,
    pub(super) messages: &'call Option<Vec<GuardrailMessage>>,
    pub(super) prompt: &'call Option<GuardrailValue>,
    pub(super) tools: &'call Option<GuardrailValue>,
    pub(super) answer: Option<&'call UpstreamAnswer>,
    pub(super) hanging: bool,
}

pub(super) struct Prepared {
    pub(super) mock: MockProvider,
    pub(super) guardrail: PangeaGuardrail,
    pub(super) data: RequestData,
    pub(super) recorder: Recorder,
}

pub(super) fn prepare(common: &Common<'_>) -> Result<Prepared, Box<HelperError>> {
    let (mock, base) = start_mock(common.answer, common.hanging)?;
    let endpoint = Endpoint {
        api_key: common.api_key,
        guardrail_name: common.guardrail_name,
        input_recipe: common.input_recipe,
        output_recipe: common.output_recipe,
        base: &base,
        hanging: common.hanging,
    };
    let guardrail = endpoint.guardrail()?;
    let data = RequestData {
        messages: common.messages.clone(),
        prompt: common.prompt.clone(),
        tools: common.tools.clone(),
    };
    Ok(Prepared {
        mock,
        guardrail,
        data,
        recorder: Recorder,
    })
}
