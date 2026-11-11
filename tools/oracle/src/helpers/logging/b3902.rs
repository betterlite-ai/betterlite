use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::langfuse_settings::Settings;
use betterlite_logging::langfuse_exporter::discard::DiscardingSpanExporter;
use betterlite_logging::langfuse_exporter::exporter::LangfuseSpanExporter;
use betterlite_logging::langfuse_exporter::options::exporter_options;
use betterlite_logging::langfuse_settings::Credentials;
use opentelemetry_sdk::trace::SpanExporter as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

use encoder::RecordingEncoder;
use journal::Journal;
use transport::{RecordingSleep, ScriptedTransport};

mod encoder;
mod journal;
mod spans;
mod transport;

pub(crate) const UNENCODABLE_KEY: &str = "test.unencodable";

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    public_key: String,
    secret_key: String,
    base_url: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    spans: Vec<spans::SpanInput>,
    #[serde(default)]
    replies: Vec<transport::Reply>,
    #[serde(default)]
    mock_mode: bool,
}

pub(crate) fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let batch: Vec<_> = input
            .spans
            .iter()
            .map(spans::span)
            .collect::<Result<_, Box<HelperError>>>()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", &error.to_string()))?;
        if input.mock_mode {
            runtime
                .block_on(DiscardingSpanExporter.export(batch))
                .map_err(|error| failure("ExportError", &error.to_string()))?;
            return Ok(json!({"result": "success", "requests": [], "sleeps": []}));
        }
        let settings = load_settings(&input.environment)?;
        let credentials = Credentials {
            public_key: Some(input.public_key.clone()),
            secret_key: Some(SecretString::from(input.secret_key.clone())),
            host: input.base_url.clone(),
        };
        let options = exporter_options(&settings, &input.environment, &credentials)
            .map_err(|error| failure("LangfuseExporterError", &error.to_string()))?;
        let journal = Arc::new(Journal::default());
        let transport = ScriptedTransport {
            replies: Mutex::new(input.replies.iter().cloned().collect()),
            journal: Arc::clone(&journal),
        };
        let encoder = Arc::new(RecordingEncoder {
            journal: Arc::clone(&journal),
        });
        let sleep = Arc::new(RecordingSleep {
            journal: Arc::clone(&journal),
        });
        let exporter = LangfuseSpanExporter::new(&options, transport, encoder, sleep)
            .map_err(|error| failure("LangfuseExporterError", &error.to_string()))?;
        let delivered = runtime.block_on(exporter.export(batch)).is_ok();
        Ok(journal.answer(delivered))
    }
}

fn load_settings(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map_err(|error| failure("ConfigError", &error.to_string()))?;
    Ok(config.sections.langfuse_settings)
}
