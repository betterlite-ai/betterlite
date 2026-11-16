use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::image_generation::config::FoundryImageConfig;

pub(super) fn dalle_gpt_model_selection() {
    dalle_names();
    mai_and_flux_names();
    inherited_names();
    priority_order();
    gpt_fallback_is_logged();
}

fn assert_selected(model: &str, expected: FoundryImageConfig) {
    assert_eq!(FoundryImageConfig::select(model), expected);
}

fn dalle_names() {
    assert_selected("", FoundryImageConfig::DallE2);
    assert_selected("DALL-E-2", FoundryImageConfig::DallE2);
    assert_selected("dall_e_2", FoundryImageConfig::DallE2);
    assert_selected("DALL-E-3", FoundryImageConfig::DallE3);
    assert_selected("dall_e_3_preview", FoundryImageConfig::DallE3);
}

fn mai_and_flux_names() {
    assert_selected("MAI-Image-2.5", FoundryImageConfig::Mai);
    assert_selected("mai_image_2_5", FoundryImageConfig::Mai);
    assert_selected("azure_ai/FLUX.2-pro", FoundryImageConfig::Flux);
    assert_selected("flux", FoundryImageConfig::Flux);
}

fn inherited_names() {
    assert_selected("gpt-image-1", FoundryImageConfig::GptImage);
    assert_selected("azure_ai/gpt-image-1", FoundryImageConfig::GptImage);
    assert_selected("dall e 3", FoundryImageConfig::GptImage);
}

fn priority_order() {
    dalle2_first();
    dalle3_second();
    mai_third();
    leftover_names();
}

fn dalle2_first() {
    assert_selected("dall-e-2-dall-e-3-flux", FoundryImageConfig::DallE2);
    assert_selected("mai-image-dall-e-2", FoundryImageConfig::DallE2);
}

fn dalle3_second() {
    assert_selected("dall-e-3-flux", FoundryImageConfig::DallE3);
    assert_selected("dall-e-3-mai-image", FoundryImageConfig::DallE3);
}

fn mai_third() {
    assert_selected("mai-image-flux", FoundryImageConfig::Mai);
}

fn leftover_names() {
    assert_selected("_", FoundryImageConfig::DallE2);
    assert_selected("DallE2x", FoundryImageConfig::DallE2);
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

struct DebugCapture(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for DebugCapture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &Level::DEBUG {
            let mut visitor = MessageVisitor(String::new());
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }
}

fn capture_debug<F, T>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(DebugCapture(Arc::clone(&messages)));
    let value = with_default(subscriber, run);
    let collected = messages.lock().unwrap().clone();
    (value, collected)
}

fn gpt_fallback_is_logged() {
    let (selected, messages) = capture_debug(|| FoundryImageConfig::select("gpt_image_1"));
    assert_eq!(selected, FoundryImageConfig::GptImage);
    assert_eq!(
        messages,
        vec![String::from("using the GPT image generation config")]
    );
    let (flux, flux_messages) = capture_debug(|| FoundryImageConfig::select("flux.2-pro"));
    assert_eq!(flux, FoundryImageConfig::Flux);
    assert!(flux_messages.is_empty());
}
