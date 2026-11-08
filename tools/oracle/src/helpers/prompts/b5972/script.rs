use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{
    ChatTemplate, Hub, TokenizerConfig, TokenizerDocument, builtin_entries,
};
use secrecy::SecretString;
use serde::Deserialize;

use crate::helpers::HelperError;

pub(super) const NOW_SECONDS: i64 = 1_700_000_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConvertCase {
    pub(crate) model: String,
    pub(crate) provider: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCase {
    pub(crate) non_default: WatsonxParams,
    #[serde(default)]
    pub(crate) optional: WatsonxParams,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) options: WatsonxParams,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlRequest {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    pub(crate) model: String,
    pub(crate) stream: bool,
    #[serde(default)]
    pub(crate) options: WatsonxParams,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyRequest {
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) now: Option<i64>,
}

pub(super) fn factory(templates: &BTreeMap<String, String>) -> PromptFactory {
    let mut cache = builtin_entries();
    for (model, template) in templates {
        cache.insert(
            model.clone(),
            TokenizerConfig::Success(Some(TokenizerDocument {
                chat_template: ChatTemplate::Text(template.clone()),
                bos_token: None,
                eos_token: None,
            })),
        );
    }
    PromptFactory {
        cache: Mutex::new(cache),
        hub: Hub::Unreachable,
    }
}

pub(super) fn environment(values: &BTreeMap<String, String>) -> WatsonxEnvironment {
    let find = |name: &str| values.get(name);
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.clone()));
    let text = |name: &str| find(name).cloned();
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: "Runtime".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}
