use betterlite_config::sections::studio_identity::Settings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_vertex::google_token_counter::counter::TokenCounter as _;
use betterlite_vertex::google_token_counter::model_info::{GeminiModelInfo, VertexModelInfo};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    providers: Vec<Option<String>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let vertex = VertexModelInfo {
            settings: VertexSettings::default(),
        }
        .get_token_counter();
        let studio = GeminiModelInfo {
            settings: Settings::default(),
        }
        .get_token_counter();
        let vertex_selected = input
            .providers
            .iter()
            .map(|provider| vertex.should_use_token_counting_api(provider.as_deref()))
            .collect::<Vec<_>>();
        let studio_selected = input
            .providers
            .iter()
            .map(|provider| studio.should_use_token_counting_api(provider.as_deref()))
            .collect::<Vec<_>>();
        Ok(json!({
            "vertex": {"kind": vertex.kind(), "selected": vertex_selected},
            "studio": {"kind": studio.kind(), "selected": studio_selected},
        }))
    }
}

mod tests;
