#![cfg(test)]

pub(crate) mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_config::sections::rate_limiter_settings;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::rate_limit::{
    ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitCategory,
};
use indexmap::IndexMap;
use rstest::rstest;

use super::router_model_groups::RouterModelGroupResolver;
use super::selection::{LimiterChoice, select_call_type_limiter};
use super::{CallTypeRateLimiter, LimiterAssemblyError, RateLimitCallData};
use crate::atomic_counter::response::ResponseDecodeError;
use crate::atomic_counter::value::CounterValueError as AtomicCounterValueError;
use crate::concurrency_gauges::GaugeError;
use crate::fail_closed::RateLimitUnverifiableError;
use crate::limiter_records::{CacheCounterValue, ModelGroupResolver};
use crate::rate_limit_window_store::WindowStoreError;
use crate::request_stash::RequestStashSlot;
use crate::settlement_and_rollback::SettlementError;
use crate::token_adjustments::store::TokenAdjustmentError;
use crate::windowed_admission::WindowedAdmissionError;
use support::{build_assembly, build_owners};

struct FailingLimiter;

#[async_trait::async_trait]
impl CallTypeRateLimiter for FailingLimiter {
    async fn async_pre_call_hook(
        &self,
        _user_api_key_dict: &UserAPIKeyAuth,
        _data: &RateLimitCallData,
        _call_type: CallType,
        _stash: &RequestStashSlot,
    ) -> Result<(), ProxyRateLimitError> {
        Err(ProxyRateLimitError::new(ProxyRateLimitErrorInit::default()).unwrap())
    }
}

#[test]
fn assembly_keeps_every_owner() {
    let settings = rate_limiter_settings::Settings::default();
    let owners = build_owners(settings);
    let assembly = build_assembly(&owners);
    assert_eq!(assembly.settings, settings);
    assert_eq!(assembly.fail_closed, owners.fail_closed);
    assert!(Arc::ptr_eq(&assembly.clock, &owners.clock));
    assert!(Arc::ptr_eq(&assembly.windows, &owners.windows));
    assert!(Arc::ptr_eq(&assembly.gauges, &owners.gauges));
    assert!(Arc::ptr_eq(&assembly.admission, &owners.admission));
    assert!(Arc::ptr_eq(&assembly.adjustments, &owners.adjustments));
    let script = assembly.check_and_increment_script.as_ref().unwrap();
    assert!(Arc::ptr_eq(script, &owners.script));
    let settlement = assembly.settlement();
    assert!(Arc::ptr_eq(
        settlement.script.as_ref().unwrap(),
        &owners.script
    ));
    assert!(Arc::ptr_eq(
        settlement.refund_script.as_ref().unwrap(),
        &owners.script
    ));
    assert_eq!(settlement.fail_closed, owners.fail_closed);
    let group = assembly.model_groups.resolve("alias");
    assert_eq!(group.as_deref(), Some("group"));
}

#[test]
fn assembly_without_a_redis_script_settles_in_memory() {
    let owners = build_owners(rate_limiter_settings::Settings::default());
    let mut assembly = build_assembly(&owners);
    assembly.check_and_increment_script = None;
    assert!(assembly.settlement().script.is_none());
}

#[test]
fn assembly_default_window_size_is_sixty() {
    let owners = build_owners(rate_limiter_settings::Settings::default());
    let assembly = build_assembly(&owners);
    assert_eq!(assembly.settings.window_size, 60);
    assert_eq!(assembly.admission.window_size, 60);
    assert_eq!(assembly.settlement().default_window_size, 60);
}

#[test]
fn assembly_custom_window_size_reaches_the_settlement() {
    let settings = rate_limiter_settings::Settings {
        fail_closed: false,
        window_size: 120,
        tpm_reservation_enabled: true,
    };
    let owners = build_owners(settings);
    let assembly = build_assembly(&owners);
    assert_eq!(assembly.settings.window_size, 120);
    assert_eq!(assembly.admission.window_size, 120);
    assert_eq!(assembly.settlement().default_window_size, 120);
}

#[tokio::test]
async fn assembly_batch_limiter_returns_the_proxy_rate_limit_error() {
    let owners = build_owners(rate_limiter_settings::Settings::default());
    let assembly = build_assembly(&owners);
    assert!(assembly.batch_limiter().is_none());
    assembly.set_batch_limiter(Arc::new(FailingLimiter));
    let limiter = assembly.batch_limiter().unwrap();
    let user = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
    let stash = RequestStashSlot::default();
    let error = limiter
        .async_pre_call_hook(&user, &RequestData::new(), CallType::AcreateBatch, &stash)
        .await
        .unwrap_err();
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
}

#[test]
fn assembly_error_converts_the_counter_errors() {
    let window_error = LimiterAssemblyError::from(WindowStoreError::MissingScript {
        name: "rate_limiter",
    });
    assert!(matches!(window_error, LimiterAssemblyError::WindowStore(_)));
    let counter_error = LimiterAssemblyError::from(AtomicCounterValueError::NotAnInteger {
        position: String::from("reply[1]"),
        value: CacheCounterValue::Float(0.5),
    });
    assert!(matches!(
        counter_error,
        LimiterAssemblyError::AtomicCounter(_)
    ));
    let response_error =
        LimiterAssemblyError::from(ResponseDecodeError::MissingElement { position: 0 });
    assert!(matches!(
        response_error,
        LimiterAssemblyError::AtomicResponse(_)
    ));
}

#[test]
fn assembly_error_converts_the_limiter_errors() {
    let settlement_error = LimiterAssemblyError::from(SettlementError::DescriptorCount {
        descriptors: 1,
        increments: 0,
    });
    assert!(matches!(
        settlement_error,
        LimiterAssemblyError::Settlement(_)
    ));
    let gauge_error = LimiterAssemblyError::from(GaugeError::MissingAcquireReply);
    assert!(matches!(gauge_error, LimiterAssemblyError::Gauge(_)));
    let admission_error =
        LimiterAssemblyError::from(WindowedAdmissionError::WindowPairCount { keys: 1, values: 2 });
    assert!(matches!(
        admission_error,
        LimiterAssemblyError::Admission(_)
    ));
    let token_error = LimiterAssemblyError::from(TokenAdjustmentError::TokenAmount { value: 1 });
    assert!(matches!(
        token_error,
        LimiterAssemblyError::TokenAdjustment(_)
    ));
    let unverifiable = RateLimitUnverifiableError {
        status_code: 503,
        detail: BTreeMap::new(),
    };
    assert!(matches!(
        LimiterAssemblyError::from(unverifiable),
        LimiterAssemblyError::Unverifiable(_)
    ));
}

#[test]
fn select_acreate_batch_chooses_the_batch_limiter() {
    let choice = select_call_type_limiter(CallType::AcreateBatch);
    assert_eq!(choice, LimiterChoice::Batch);
}

#[rstest]
#[case::embedding(CallType::Embedding)]
#[case::aembedding(CallType::Aembedding)]
#[case::completion(CallType::Completion)]
#[case::acompletion(CallType::Acompletion)]
#[case::atext_completion(CallType::AtextCompletion)]
#[case::text_completion(CallType::TextCompletion)]
#[case::image_generation(CallType::ImageGeneration)]
#[case::aimage_generation(CallType::AimageGeneration)]
#[case::image_edit(CallType::ImageEdit)]
#[case::aimage_edit(CallType::AimageEdit)]
#[case::moderation(CallType::Moderation)]
#[case::amoderation(CallType::Amoderation)]
#[case::atranscription(CallType::Atranscription)]
#[case::transcription(CallType::Transcription)]
#[case::aspeech(CallType::Aspeech)]
#[case::speech(CallType::Speech)]
#[case::rerank(CallType::Rerank)]
#[case::arerank(CallType::Arerank)]
#[case::search(CallType::Search)]
#[case::asearch(CallType::Asearch)]
#[case::decisions(CallType::Decisions)]
#[case::adecisions(CallType::Adecisions)]
#[case::systemone(CallType::Systemone)]
#[case::asystemone(CallType::Asystemone)]
#[case::private_arealtime(CallType::PrivateArealtime)]
#[case::private_aresponses_websocket(CallType::PrivateAresponsesWebsocket)]
#[case::create_batch(CallType::CreateBatch)]
#[case::aretrieve_batch(CallType::AretrieveBatch)]
#[case::retrieve_batch(CallType::RetrieveBatch)]
#[case::acancel_batch(CallType::AcancelBatch)]
#[case::cancel_batch(CallType::CancelBatch)]
#[case::pass_through_endpoint(CallType::PassThroughEndpoint)]
#[case::anthropic_messages(CallType::AnthropicMessages)]
#[case::aanthropic_messages(CallType::AanthropicMessages)]
#[case::get_assistants(CallType::GetAssistants)]
#[case::aget_assistants(CallType::AgetAssistants)]
#[case::create_assistants(CallType::CreateAssistants)]
#[case::acreate_assistants(CallType::AcreateAssistants)]
#[case::delete_assistant(CallType::DeleteAssistant)]
#[case::adelete_assistant(CallType::AdeleteAssistant)]
#[case::acreate_thread(CallType::AcreateThread)]
#[case::create_thread(CallType::CreateThread)]
#[case::aget_thread(CallType::AgetThread)]
#[case::get_thread(CallType::GetThread)]
#[case::aadd_message(CallType::AAddMessage)]
#[case::add_message(CallType::AddMessage)]
#[case::aget_messages(CallType::AgetMessages)]
#[case::get_messages(CallType::GetMessages)]
#[case::arun_thread(CallType::ArunThread)]
#[case::run_thread(CallType::RunThread)]
#[case::arun_thread_stream(CallType::ArunThreadStream)]
#[case::run_thread_stream(CallType::RunThreadStream)]
#[case::afile_retrieve(CallType::AfileRetrieve)]
#[case::file_retrieve(CallType::FileRetrieve)]
#[case::afile_delete(CallType::AfileDelete)]
#[case::file_delete(CallType::FileDelete)]
#[case::afile_list(CallType::AfileList)]
#[case::file_list(CallType::FileList)]
#[case::acreate_file(CallType::AcreateFile)]
#[case::create_file(CallType::CreateFile)]
#[case::afile_content(CallType::AfileContent)]
#[case::file_content(CallType::FileContent)]
#[case::create_fine_tuning_job(CallType::CreateFineTuningJob)]
#[case::acreate_fine_tuning_job(CallType::AcreateFineTuningJob)]
#[case::create_video(CallType::CreateVideo)]
#[case::acreate_video(CallType::AcreateVideo)]
#[case::video_generation(CallType::VideoGeneration)]
#[case::avideo_generation(CallType::AvideoGeneration)]
#[case::avideo_retrieve(CallType::AvideoRetrieve)]
#[case::video_retrieve(CallType::VideoRetrieve)]
#[case::avideo_content(CallType::AvideoContent)]
#[case::video_content(CallType::VideoContent)]
#[case::video_remix(CallType::VideoRemix)]
#[case::avideo_remix(CallType::AvideoRemix)]
#[case::video_list(CallType::VideoList)]
#[case::avideo_list(CallType::AvideoList)]
#[case::video_retrieve_job(CallType::VideoRetrieveJob)]
#[case::avideo_retrieve_job(CallType::AvideoRetrieveJob)]
#[case::video_delete(CallType::VideoDelete)]
#[case::avideo_delete(CallType::AvideoDelete)]
#[case::video_create_character(CallType::VideoCreateCharacter)]
#[case::avideo_create_character(CallType::AvideoCreateCharacter)]
#[case::video_get_character(CallType::VideoGetCharacter)]
#[case::avideo_get_character(CallType::AvideoGetCharacter)]
#[case::video_edit(CallType::VideoEdit)]
#[case::avideo_edit(CallType::AvideoEdit)]
#[case::video_extension(CallType::VideoExtension)]
#[case::avideo_extension(CallType::AvideoExtension)]
#[case::vector_store_file_create(CallType::VectorStoreFileCreate)]
#[case::avector_store_file_create(CallType::AvectorStoreFileCreate)]
#[case::vector_store_file_list(CallType::VectorStoreFileList)]
#[case::avector_store_file_list(CallType::AvectorStoreFileList)]
#[case::vector_store_file_retrieve(CallType::VectorStoreFileRetrieve)]
#[case::avector_store_file_retrieve(CallType::AvectorStoreFileRetrieve)]
#[case::vector_store_file_content(CallType::VectorStoreFileContent)]
#[case::avector_store_file_content(CallType::AvectorStoreFileContent)]
#[case::vector_store_file_update(CallType::VectorStoreFileUpdate)]
#[case::avector_store_file_update(CallType::AvectorStoreFileUpdate)]
#[case::vector_store_file_delete(CallType::VectorStoreFileDelete)]
#[case::avector_store_file_delete(CallType::AvectorStoreFileDelete)]
#[case::vector_store_create(CallType::VectorStoreCreate)]
#[case::avector_store_create(CallType::AvectorStoreCreate)]
#[case::vector_store_search(CallType::VectorStoreSearch)]
#[case::avector_store_search(CallType::AvectorStoreSearch)]
#[case::ingest(CallType::Ingest)]
#[case::aingest(CallType::Aingest)]
#[case::query(CallType::Query)]
#[case::aquery(CallType::Aquery)]
#[case::create_interaction(CallType::CreateInteraction)]
#[case::acreate_interaction(CallType::AcreateInteraction)]
#[case::create_container(CallType::CreateContainer)]
#[case::acreate_container(CallType::AcreateContainer)]
#[case::list_containers(CallType::ListContainers)]
#[case::alist_containers(CallType::AlistContainers)]
#[case::retrieve_container(CallType::RetrieveContainer)]
#[case::aretrieve_container(CallType::AretrieveContainer)]
#[case::delete_container(CallType::DeleteContainer)]
#[case::adelete_container(CallType::AdeleteContainer)]
#[case::list_container_files(CallType::ListContainerFiles)]
#[case::alist_container_files(CallType::AlistContainerFiles)]
#[case::upload_container_file(CallType::UploadContainerFile)]
#[case::aupload_container_file(CallType::AuploadContainerFile)]
#[case::create_sandbox(CallType::CreateSandbox)]
#[case::acreate_sandbox(CallType::AcreateSandbox)]
#[case::delete_sandbox(CallType::DeleteSandbox)]
#[case::adelete_sandbox(CallType::AdeleteSandbox)]
#[case::run_code(CallType::RunCode)]
#[case::arun_code(CallType::ArunCode)]
#[case::code_interpreter_tool(CallType::CodeInterpreterTool)]
#[case::acode_interpreter_tool(CallType::AcodeInterpreterTool)]
#[case::acancel_fine_tuning_job(CallType::AcancelFineTuningJob)]
#[case::cancel_fine_tuning_job(CallType::CancelFineTuningJob)]
#[case::alist_fine_tuning_jobs(CallType::AlistFineTuningJobs)]
#[case::list_fine_tuning_jobs(CallType::ListFineTuningJobs)]
#[case::aretrieve_fine_tuning_job(CallType::AretrieveFineTuningJob)]
#[case::retrieve_fine_tuning_job(CallType::RetrieveFineTuningJob)]
#[case::responses(CallType::Responses)]
#[case::aresponses(CallType::Aresponses)]
#[case::alist_input_items(CallType::AlistInputItems)]
#[case::llm_passthrough_route(CallType::LlmPassthroughRoute)]
#[case::allm_passthrough_route(CallType::AllmPassthroughRoute)]
#[case::generate_content(CallType::GenerateContent)]
#[case::agenerate_content(CallType::AgenerateContent)]
#[case::generate_content_stream(CallType::GenerateContentStream)]
#[case::agenerate_content_stream(CallType::AgenerateContentStream)]
#[case::ocr(CallType::Ocr)]
#[case::aocr(CallType::Aocr)]
#[case::call_mcp_tool(CallType::CallMcpTool)]
#[case::list_mcp_tools(CallType::ListMcpTools)]
#[case::asend_message(CallType::AsendMessage)]
#[case::send_message(CallType::SendMessage)]
#[case::acreate_skill(CallType::AcreateSkill)]
fn select_other_call_types_choose_the_main_limiter(#[case] call_type: CallType) {
    let choice = select_call_type_limiter(call_type);
    assert_eq!(choice, LimiterChoice::Main);
}

fn build_resolver(aliases: &[(&str, &str)]) -> RouterModelGroupResolver {
    let table: IndexMap<String, GroupAlias> = aliases
        .iter()
        .map(|(alias, target)| {
            (
                String::from(*alias),
                GroupAlias {
                    model: String::from(*target),
                    hidden: false,
                },
            )
        })
        .collect();
    RouterModelGroupResolver {
        aliases: Arc::new(table),
    }
}

#[test]
fn resolver_returns_the_group_for_an_alias() {
    let resolver = build_resolver(&[("gpt4", "gpt-4")]);
    let resolved = resolver.resolve("gpt4");
    assert_eq!(resolved.as_deref(), Some("gpt-4"));
}

#[test]
fn resolver_returns_a_target_that_matches_the_name() {
    let resolver = build_resolver(&[("gpt4", "gpt4")]);
    let resolved = resolver.resolve("gpt4");
    assert_eq!(resolved.as_deref(), Some("gpt4"));
}

#[test]
fn resolver_returns_nothing_for_a_group_name() {
    let resolver = build_resolver(&[("gpt4", "gpt-4")]);
    let resolved = resolver.resolve("gpt-4");
    assert_eq!(resolved, None);
}

#[test]
fn resolver_returns_nothing_for_an_unknown_name() {
    let resolver = build_resolver(&[("gpt4", "gpt-4")]);
    let resolved = resolver.resolve("unknown");
    assert_eq!(resolved, None);
}

#[test]
fn resolver_returns_nothing_for_an_empty_target() {
    let resolver = build_resolver(&[("gpt4", "")]);
    let resolved = resolver.resolve("gpt4");
    assert_eq!(resolved, None);
}
