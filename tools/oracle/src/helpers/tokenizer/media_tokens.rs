use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_tokenizer::count::messages;
use betterlite_tokenizer::count::request::Message;
use betterlite_tokenizer::count::settings::Settings as CountSettings;
use betterlite_tokenizer::count::text::{Counted as TextCounted, Texter};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use betterlite_tokenizer::image::settings::Settings as ImageSettings;
use betterlite_tokenizer::limits::{self, Models, Settings as LimitSettings};
use betterlite_tokenizer::media::count::count_token_media;
use betterlite_tokenizer::media::settings::{ImageCounting, Settings as MediaSettings};
use betterlite_tokenizer::media::wire::TokenMedia;
use betterlite_tokenizer::selection::openai_tokenizer_encoding_name;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const DEFAULT_MODEL: &str = "gpt-3.5-turbo";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    settings: ImageSettings,
    #[serde(default)]
    model: Option<String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    CountContent {
        content: Vec<Value>,
        #[serde(default)]
        use_default_image_token_count: bool,
        #[serde(default)]
        default_token_count: Option<u64>,
    },
    ModifiedMaxTokens {
        model: String,
        base_model: String,
        #[serde(default)]
        messages: Option<Vec<Message>>,
        #[serde(default)]
        user_max_tokens: Option<i64>,
        #[serde(default)]
        buffer_perc: Option<f64>,
        #[serde(default)]
        buffer_num: Option<f64>,
    },
}

struct DirectRunner;

#[async_trait]
impl BlockingCounter for DirectRunner {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

struct Fixture {
    service: Arc<CountingService>,
    counted: TextCounted,
    max_exact_chars: NonZeroUsize,
}

impl Fixture {
    fn texter(&self) -> Texter<'_> {
        Texter {
            counter: self.service.as_ref(),
            counted: &self.counted,
            max_exact_chars: self.max_exact_chars,
        }
    }
}

fn fixture(encoding: Encoding) -> Result<Fixture, Box<HelperError>> {
    let settings = CountSettings::default();
    let limits = settings
        .encode_limits()
        .map_err(|error| failure(error.to_string()))?;
    let service = Arc::new(CountingService::new(4, Arc::new(DirectRunner), limits));
    let data = assets::bundled(encoding)
        .ok_or_else(|| failure(format!("no bundled vocabulary for {}", encoding.name())))?;
    let engine = TiktokenEngine::from_bpe_file(encoding, &data)
        .map_err(|error| failure(error.to_string()))?;
    Ok(Fixture {
        service,
        counted: TextCounted {
            engine: Engine::Tiktoken(Arc::new(engine)),
            chunks: Some(settings.chunk_chars),
        },
        max_exact_chars: settings.max_exact_chars,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        runtime.block_on(run_calls(input))
    }
}

async fn run_calls(input: &Input) -> HelperResult {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let model = input.model.as_deref().unwrap_or(DEFAULT_MODEL);
    let mut values = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        values.push(run_call(call, &catalog, model, &input.settings).await?);
    }
    Ok(Value::Array(values))
}

struct CountCall<'call> {
    content: &'call [Value],
    uses_default: bool,
    default_token_count: Option<u64>,
}

struct ModifiedCall<'call> {
    model: &'call str,
    base_model: &'call str,
    messages: Option<&'call [Message]>,
    user_max_tokens: Option<i64>,
    buffer_perc: Option<f64>,
    buffer_num: Option<f64>,
}

async fn run_call(
    call: &Call,
    catalog: &ModelCatalog,
    content_model: &str,
    image: &ImageSettings,
) -> HelperResult {
    match call {
        Call::CountContent {
            content,
            use_default_image_token_count,
            default_token_count,
        } => {
            let request = CountCall {
                content,
                uses_default: *use_default_image_token_count,
                default_token_count: *default_token_count,
            };
            count_call(request, catalog, content_model, image).await
        }
        Call::ModifiedMaxTokens {
            model,
            base_model,
            messages,
            user_max_tokens,
            buffer_perc,
            buffer_num,
        } => {
            let request = ModifiedCall {
                model,
                base_model,
                messages: messages.as_deref(),
                user_max_tokens: *user_max_tokens,
                buffer_perc: *buffer_perc,
                buffer_num: *buffer_num,
            };
            modified_call(request, catalog).await
        }
    }
}

async fn count_call(
    request: CountCall<'_>,
    catalog: &ModelCatalog,
    model: &str,
    image: &ImageSettings,
) -> HelperResult {
    let encoding = openai_tokenizer_encoding_name(model, catalog);
    let fixture = fixture(encoding)?;
    let content = request
        .content
        .iter()
        .map(to_raw_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| failure(error.to_string()))?;
    let media = TokenMedia { content };
    let settings = MediaSettings {
        image: *image,
        fallback_token_count: request.default_token_count,
        image_counting: if request.uses_default {
            ImageCounting::DefaultCount
        } else {
            ImageCounting::Exact
        },
    };
    let counted = count_token_media(&fixture.texter(), &media, &settings)
        .await
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!(counted.tokens))
}

async fn modified_call(request: ModifiedCall<'_>, catalog: &ModelCatalog) -> HelperResult {
    let encoding = openai_tokenizer_encoding_name(request.base_model, catalog);
    let fixture = fixture(encoding)?;
    let input_tokens = match request.messages {
        Some(messages) => input_tokens(&fixture, request.base_model, catalog, messages)
            .await
            .ok(),
        None => None,
    };
    let settings = LimitSettings {
        buffer_fraction: request
            .buffer_perc
            .unwrap_or(limits::DEFAULT_BUFFER_FRACTION),
        buffer_minimum_tokens: request
            .buffer_num
            .unwrap_or(limits::DEFAULT_BUFFER_MINIMUM_TOKENS),
    };
    let modified = limits::get_modified_max_tokens(
        catalog,
        Models {
            model: request.model,
            base_model: request.base_model,
        },
        request.user_max_tokens,
        input_tokens,
        &settings,
    );
    Ok(json!(modified.max_tokens))
}

async fn input_tokens(
    fixture: &Fixture,
    base_model: &str,
    catalog: &ModelCatalog,
    messages: &[Message],
) -> Result<usize, Box<HelperError>> {
    let texter = fixture.texter();
    let params = messages::message_params(base_model, catalog);
    let counted = messages::count_messages(&texter, params, messages)
        .await
        .map_err(|error| failure(error.to_string()))?;
    let includes_system = messages.iter().any(|message| message.role == "system");
    let extra = messages::count_extra(&texter, None, None, includes_system)
        .await
        .map_err(|error| failure(error.to_string()))?;
    counted
        .checked_add(extra)
        .ok_or_else(|| failure("the token count does not fit in a machine word".to_owned()))
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "MediaError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
