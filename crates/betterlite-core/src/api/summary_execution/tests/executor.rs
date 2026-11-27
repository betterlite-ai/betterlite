use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::sections::mock_response::selection::MockPermission;
use betterlite_config::sections::mock_response::{MockResponse, UsageDefaults};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, Provider};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use secrecy::SecretString;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::time::sleep;
use url::Url;
use uuid::Uuid;

use crate::api::messages_context_clear::constants::COMPACT_SUMMARY_SYSTEM_PREFIX;
use crate::api::messages_context_clear::result::CompactionKind;
use crate::api::summary_execution::completion::ExecutorCompletion;

use super::support::{Fixture, Port, Sample, capture_logs, run};

const MODEL: &str = "main-model";
const MOCK_ID: &str = "sum-mock";

fn fixed_clock() -> u64 {
    1_700_000_000
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn outbound() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::outbound(&policy).expect("the outbound client builds")
}

fn executor(responses: BTreeMap<String, MockResponse>, usage: UsageDefaults) -> ChatExecutor {
    ChatExecutor {
        client: outbound(),
        deployments: Vec::new(),
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(7)),
        clock: fixed_clock,
        next_id: fixed_id,
        mock: MockSettings {
            responses,
            usage,
            permission: MockPermission::default(),
        },
        mock_chat: crate::mock::chat::response,
        mock_embedding: crate::mock::embedding::response,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        bundled_catalog: Arc::default(),
    }
}

fn deployment(model_id: &str, api_base: &str) -> ChatDeployment {
    ChatDeployment {
        model_group: String::from("sum"),
        litellm_model: String::from("openai/gpt-4o-mini"),
        provider: Provider::OpenAi {
            model: String::from("gpt-4o-mini"),
        },
        api_base: api_base.to_owned(),
        endpoint: Url::parse(&format!("{api_base}/chat/completions"))
            .expect("the endpoint is a URL"),
        authorization: None,
        api_key: Some(SecretString::from("sk-test")),
        organization: None,
        timeout: Duration::from_secs(6000),
        timeout_seconds: 6000.0,
        model_id: model_id.to_owned(),
        weight: 0,
        rpm: 0,
        tpm: 0,
        global_drop_params: false,
        drop_policy: DeploymentDropPolicy::default(),
    }
}

fn mock_answers(answer: &str) -> BTreeMap<String, MockResponse> {
    BTreeMap::from([(String::from(MOCK_ID), MockResponse::Text(answer.to_owned()))])
}

#[tokio::test]
async fn a_mock_provider_executor_runs_the_summary_end_to_end() {
    let usage = UsageDefaults {
        prompt_tokens: 12,
        completion_tokens: 3,
    };
    let executor = executor(mock_answers("<summary> NEW </summary>"), usage);
    let deployment = deployment(MOCK_ID, "http://127.0.0.1:9/v1");
    let completion = ExecutorCompletion {
        executor: &executor,
        deployment: &deployment,
    };
    let fixture = Fixture::default();
    let result = run(
        &fixture,
        Sample::default().request(MODEL),
        Port::Direct(&completion),
    )
    .await
    .expect("the editor runs");
    let block = result.compaction_block.clone().expect("a compaction block");
    assert_eq!(block.kind, CompactionKind::Compaction);
    assert_eq!(block.content.as_deref(), Some("NEW"));
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.summary_input_tokens, Some(12));
    assert_eq!(edit.summary_output_tokens, Some(3));
    let system = format!("{COMPACT_SUMMARY_SYSTEM_PREFIX}NEW\n\nBASE SYSTEM");
    assert_eq!(result.system, Some(DecisionJson::Text(system)));
}

#[tokio::test]
async fn a_mock_provider_failure_is_a_summary_call_failure() {
    let executor = executor(
        mock_answers("litellm.RateLimitError"),
        UsageDefaults::default(),
    );
    let deployment = deployment(MOCK_ID, "http://127.0.0.1:9/v1");
    let completion = ExecutorCompletion {
        executor: &executor,
        deployment: &deployment,
    };
    let fixture = Fixture::default();
    let (logs, _guard) = capture_logs();
    let result = run(
        &fixture,
        Sample::default().request(MODEL),
        Port::Direct(&completion),
    )
    .await
    .expect("the editor runs");
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.error.as_deref(), Some("summary_call_failed"));
    assert!(result.compaction_block.is_none());
    assert!(logs.contains("summary call failed: the provider answered with status 429"));
}

#[tokio::test]
async fn a_max_tokens_past_u32_fails_the_summary_call() {
    let executor = executor(
        mock_answers("<summary> NEW </summary>"),
        UsageDefaults::default(),
    );
    let deployment = deployment(MOCK_ID, "http://127.0.0.1:9/v1");
    let completion = ExecutorCompletion {
        executor: &executor,
        deployment: &deployment,
    };
    let mut fixture = Fixture::default();
    let settings = fixture.settings.as_mut().expect("one summary settings");
    settings.context_management_summary_max_tokens = u64::from(u32::MAX).saturating_add(1);
    let (logs, _guard) = capture_logs();
    let result = run(
        &fixture,
        Sample::default().request(MODEL),
        Port::Direct(&completion),
    )
    .await
    .expect("the editor runs");
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.error.as_deref(), Some("summary_call_failed"));
    assert!(
        logs.contains(
            "summary call failed: the request body is not a valid chat completion request"
        )
    );
}

async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(read, 0, "the client sends its request before the deadline");
    sleep(Duration::from_secs(30)).await;
}

#[tokio::test(start_paused = true)]
async fn a_stalled_provider_reaches_the_sixty_second_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let executor = executor(BTreeMap::new(), UsageDefaults::default());
    let deployment = deployment("no-mock", &format!("http://{address}/v1"));
    let completion = ExecutorCompletion {
        executor: &executor,
        deployment: &deployment,
    };
    let fixture = Fixture::default();
    let (logs, _guard) = capture_logs();
    let started = tokio::time::Instant::now();
    let (outcome, ()) = tokio::join!(
        async {
            let outcome = run(
                &fixture,
                Sample::default().request(MODEL),
                Port::Direct(&completion),
            )
            .await;
            (outcome, started.elapsed())
        },
        hold_open(listener),
    );
    let (result, elapsed) = outcome;
    let result = result.expect("the editor runs");
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.error.as_deref(), Some("summary_call_failed"));
    assert!(logs.contains(
        "summary call failed: the provider did not answer within the deployment timeout"
    ));
    assert_eq!(elapsed, Duration::from_secs(60));
}
