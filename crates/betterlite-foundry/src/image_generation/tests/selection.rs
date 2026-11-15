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
    other_names();
    priority_order();
    gpt_fallback_is_logged();
}

fn dalle_names() {
    assert_eq!(FoundryImageConfig::select(""), FoundryImageConfig::DallE2);
    assert_eq!(
        FoundryImageConfig::select("DALL-E-2"),
        FoundryImageConfig::DallE2
    );
    assert_eq!(
        FoundryImageConfig::select("dall_e_2"),
        FoundryImageConfig::DallE2
    );
    assert_eq!(
        FoundryImageConfig::select("DALL-E-3"),
        FoundryImageConfig::DallE3
    );
    assert_eq!(
        FoundryImageConfig::select("dall_e_3_preview"),
        FoundryImageConfig::DallE3
    );
}

fn other_names() {
    assert_eq!(
        FoundryImageConfig::select("MAI-Image-2.5"),
        FoundryImageConfig::Mai
    );
    assert_eq!(
        FoundryImageConfig::select("mai_image_2_5"),
        FoundryImageConfig::Mai
    );
    assert_eq!(
        FoundryImageConfig::select("azure_ai/FLUX.2-pro"),
        FoundryImageConfig::Flux
    );
    assert_eq!(FoundryImageConfig::select("flux"), FoundryImageConfig::Flux);
    assert_eq!(
        FoundryImageConfig::select("gpt-image-1"),
        FoundryImageConfig::GptImage
    );
    assert_eq!(
        FoundryImageConfig::select("azure_ai/gpt-image-1"),
        FoundryImageConfig::GptImage
    );
    assert_eq!(
        FoundryImageConfig::select("dall e 3"),
        FoundryImageConfig::GptImage
    );
}

fn priority_order() {
    assert_eq!(
        FoundryImageConfig::select("dall-e-2-dall-e-3-flux"),
        FoundryImageConfig::DallE2
    );
    assert_eq!(
        FoundryImageConfig::select("dall-e-3-flux"),
        FoundryImageConfig::DallE3
    );
    assert_eq!(
        FoundryImageConfig::select("mai-image-flux"),
        FoundryImageConfig::Mai
    );
    assert_eq!(FoundryImageConfig::select("_"), FoundryImageConfig::DallE2);
    assert_eq!(
        FoundryImageConfig::select("DallE2x"),
        FoundryImageConfig::DallE2
    );
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
    let (selected, messages) = capture_debug(|| FoundryImageConfig::select("flux.2-pro"));
    assert_eq!(selected, FoundryImageConfig::Flux);
    assert!(messages.is_empty());
}
