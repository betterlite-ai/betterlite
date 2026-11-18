use std::error::Error as StdError;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_batching::alert_delivery::PROXY_BASE_URL_ENV;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::analytics::python::number_text;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::ModelAlertSender;

const DEFAULT_PROXY_BASE_URL: &str = "http://0.0.0.0:4000";
const INPUT_COST_KEY: &str = "input_cost_per_token";
const OUTPUT_COST_KEY: &str = "output_cost_per_token";

pub trait RawModelInfoSource {
    fn model_info(&self, name: &str) -> Option<&JsonObject>;
}

#[derive(Debug, thiserror::Error)]
pub enum ModelInfoRenderError {
    #[error("the model info field {field} is not a number")]
    Cost { field: String },
    #[error("a model info value could not be written as JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ModelAddedAlertError<E>
where
    E: StdError + 'static,
{
    #[error("the new model alert message could not be rendered")]
    Render(#[from] ModelInfoRenderError),
    #[error("the new model alert could not be sent")]
    Alert(#[source] E),
}

pub struct ModelAddedAlerts<Catalog, Sender, Environment> {
    pub catalog: Catalog,
    pub sender: Sender,
    pub environment: Environment,
}

impl<Catalog, Sender, Environment> ModelAddedAlerts<Catalog, Sender, Environment>
where
    Catalog: RawModelInfoSource + Send + Sync,
    Sender: ModelAlertSender,
    Environment: EnvironmentLookup + Send + Sync,
{
    pub async fn model_added_alert(
        &self,
        model_name: &str,
        litellm_model_name: &str,
        base_model: Option<&str>,
    ) -> Result<(), ModelAddedAlertError<Sender::Error>> {
        let proxy_base_url = self.environment.lookup(PROXY_BASE_URL_ENV);
        let message = render_model_added_message(
            model_name,
            litellm_model_name,
            base_model,
            &self.catalog,
            proxy_base_url.as_deref(),
        )?;
        let metadata = IndexMap::new();
        let alert = Alert {
            message: &message,
            level: AlertLevel::Low,
            alert_type: AlertType::NewModelAdded,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &metadata,
        };
        self.sender
            .send_alert(alert)
            .await
            .map_err(ModelAddedAlertError::Alert)
    }
}

#[must_use]
pub fn render_model_added_message(
    model_name: &str,
    litellm_model_name: &str,
    base_model: Option<&str>,
    catalog: &dyn RawModelInfoSource,
    proxy_base_url: Option<&str>,
) -> Result<String, ModelInfoRenderError> {
    let base_line = match base_model {
        Some(name) => format!("Base Model: `{name}`\n"),
        None => String::new(),
    };
    let model_info = match catalog.model_info(base_model.unwrap_or(litellm_model_name)) {
        Some(entries) => render_model_info(entries)?,
        None => String::new(),
    };
    let base_url = proxy_base_url.unwrap_or(DEFAULT_PROXY_BASE_URL);
    Ok(format!(
        "\n*\u{1f685} New Model Added*\nModel Name: `{model_name}`\n{base_line}\n\nUsage OpenAI Python SDK:\n```\nimport openai\nclient = openai.OpenAI(\n    api_key=\"your_api_key\",\n    base_url={base_url}\n)\n\nresponse = client.chat.completions.create(\n    model=\"{model_name}\", # model to send to the proxy\n    messages = [\n        {{\n            \"role\": \"user\",\n            \"content\": \"this is a test request, write a short poem\"\n        }}\n    ]\n)\n```\n\nModel Info: \n```\n{model_info}\n```\n"
    ))
}

fn render_model_info(entries: &JsonObject) -> Result<String, ModelInfoRenderError> {
    let mut rendered = String::new();
    for (key, value) in entries {
        rendered.push_str(key);
        rendered.push_str(": ");
        rendered.push_str(&render_model_info_value(key, value)?);
        rendered.push('\n');
    }
    Ok(rendered)
}

fn render_model_info_value(key: &str, value: &Json) -> Result<String, ModelInfoRenderError> {
    if key == INPUT_COST_KEY || key == OUTPUT_COST_KEY {
        return render_cost(key, value);
    }
    match value {
        Json::Null => Ok("None".to_owned()),
        Json::Bool(true) => Ok("True".to_owned()),
        Json::Bool(false) => Ok("False".to_owned()),
        Json::Number(number) => Ok(number_text(number)),
        Json::Text(text) => Ok(text.clone()),
        Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).map_err(|source| ModelInfoRenderError::Json { source })
        }
    }
}

fn render_cost(field: &str, value: &Json) -> Result<String, ModelInfoRenderError> {
    let number = match value {
        Json::Number(number) => number.as_f64().ok_or_else(|| ModelInfoRenderError::Cost {
            field: field.to_owned(),
        })?,
        Json::Bool(flag) => f64::from(u8::from(*flag)),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            return Err(ModelInfoRenderError::Cost {
                field: field.to_owned(),
            });
        }
    };
    Ok(format!("{number:.8}"))
}

#[cfg(test)]
mod tests;
