use betterlite_models::decisions::{Extra, Json};

// The four asynchronous paths LiteLLM's completion selects, plus the vision route that has no SDK call
#[derive(Clone, Debug)]
pub enum LegacyInvocation {
    Chat {
        model: String,
        prompt: String,
        options: Extra,
    },
    Text {
        model: String,
        prompt: String,
        options: Extra,
    },
    CustomEndpoint {
        model: String,
        prompt: String,
        instances: Vec<Json>,
        options: Extra,
    },
    PrivateEndpoint {
        model: String,
        prompt: String,
        endpoint: String,
        instances: Vec<Json>,
        options: Extra,
    },
    Vision {
        model: String,
        prompt: String,
    },
}

impl LegacyInvocation {
    #[must_use]
    pub fn prompt(&self) -> &str {
        match self {
            Self::Chat { prompt, .. }
            | Self::Text { prompt, .. }
            | Self::CustomEndpoint { prompt, .. }
            | Self::PrivateEndpoint { prompt, .. }
            | Self::Vision { prompt, .. } => prompt,
        }
    }

    #[must_use]
    pub fn model(&self) -> &str {
        match self {
            Self::Chat { model, .. }
            | Self::Text { model, .. }
            | Self::CustomEndpoint { model, .. }
            | Self::PrivateEndpoint { model, .. }
            | Self::Vision { model, .. } => model,
        }
    }
}

pub fn remove_instance_stream(instances: &mut [Json]) {
    let Some(Json::Object(instance)) = instances.first_mut() else {
        return;
    };
    instance.shift_remove("stream");
}
