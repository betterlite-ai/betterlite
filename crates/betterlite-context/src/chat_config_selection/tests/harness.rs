use betterlite_models::identity::provider_id::ProviderId;

use crate::bedrock_config_selection::chat::BedrockRoute;

use super::super::facts::{BedrockFacts, ModelFamilyProbe};
use super::super::json_provider::{JsonProviderLookup, JsonProviderProjection};
use super::super::selection::{ChatConfigError, ChatConfigRequest, select_chat_config};
use super::super::table::ProviderConfigTable;
use super::super::tags::{ChatConfigTag, ConfigConstruction, ConfigSelection};
use super::super::vertex::VertexModelLists;

pub(super) const CHAT_CATALOG: &[&str] = &[
    "o1",
    "o3-mini",
    "o4-mini",
    "gpt-4o",
    "gpt-5",
    "gpt-5-chat-latest",
    "gpt-6",
    "gpt-4o-mini",
];
pub(super) const TEXT_CATALOG: &[&str] = &["gpt-3.5-turbo-instruct"];

pub(super) struct Probe {
    chat_models: Vec<&'static str>,
    text_models: Vec<&'static str>,
}

impl Probe {
    fn catalog(&self, model: &str) -> bool {
        let checked = model
            .split_once("responses/")
            .map_or(model, |(_, suffix)| suffix);
        self.chat_models.contains(&checked) || self.text_models.contains(&checked)
    }
}

impl ModelFamilyProbe for Probe {
    fn openai_o_series(&self, model: &str) -> bool {
        let name = model.rsplit('/').next().unwrap_or(model);
        let mut chars = name.chars();
        chars.next() == Some('o')
            && chars.next().is_some_and(|second| second.is_ascii_digit())
            && self.chat_models.contains(&name)
    }

    fn openai_gpt_5(&self, model: &str) -> bool {
        let name = model.rsplit('/').next().unwrap_or(model);
        (model.contains("gpt-5") || model.contains("gpt-6")) && !name.contains("gpt-5-chat")
    }

    fn openai_catalog_model(&self, model: &str) -> bool {
        self.catalog(model)
    }

    fn azure_gpt_5(&self, model: &str) -> bool {
        let name = model.rsplit('/').next().unwrap_or(model);
        ((model.contains("gpt-5") || model.contains("gpt-6")) && !name.contains("gpt-5-chat"))
            || model.contains("gpt5_series")
    }
}

impl JsonProviderProjection for Vec<(ProviderId, JsonProviderLookup)> {
    fn lookup(&self, provider: ProviderId) -> JsonProviderLookup {
        self.iter()
            .find(|(entry, _)| *entry == provider)
            .map_or(JsonProviderLookup::Absent, |(_, lookup)| *lookup)
    }
}

pub(super) const DEFAULT_BEDROCK: BedrockFacts<'static> = BedrockFacts {
    route: BedrockRoute::Invoke,
    invoke_provider: None,
    base_model: "test-model",
};

pub(super) struct CallSpec<'source> {
    pub(super) provider: ProviderId,
    pub(super) model: &'source str,
    pub(super) base_model: Option<&'source str>,
    pub(super) bedrock: BedrockFacts<'source>,
}

impl CallSpec<'_> {
    pub(super) const fn plain(provider: ProviderId, model: &'static str) -> CallSpec<'static> {
        CallSpec {
            provider,
            model,
            base_model: None,
            bedrock: DEFAULT_BEDROCK,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Arguments {
    Shared,
    None,
    Model,
}

#[derive(Debug)]
pub(super) struct Case {
    pub(super) provider: ProviderId,
    pub(super) model: &'static str,
    pub(super) base_model: Option<&'static str>,
    pub(super) tag: ChatConfigTag,
    pub(super) arguments: Arguments,
}

pub(super) struct World {
    probe: Probe,
    mistral_models: Vec<String>,
    ai21_models: Vec<String>,
    json: Vec<(ProviderId, JsonProviderLookup)>,
}

impl World {
    pub(super) fn select(
        &self,
        spec: &CallSpec<'_>,
    ) -> Result<Option<ConfigSelection<ChatConfigTag>>, ChatConfigError> {
        let mut table = ProviderConfigTable::default();
        self.select_with(spec, &mut table)
    }

    pub(super) fn select_with(
        &self,
        spec: &CallSpec<'_>,
        table: &mut ProviderConfigTable,
    ) -> Result<Option<ConfigSelection<ChatConfigTag>>, ChatConfigError> {
        let request = ChatConfigRequest {
            model: spec.model,
            provider: spec.provider,
            base_model: spec.base_model,
            families: &self.probe,
            bedrock: spec.bedrock,
            vertex: VertexModelLists {
                mistral_models: &self.mistral_models,
                ai21_models: &self.ai21_models,
            },
            json_providers: &self.json,
        };
        select_chat_config(&request, table)
    }

    pub(super) fn set_vertex_lists(
        &mut self,
        mistral_models: Vec<String>,
        ai21_models: Vec<String>,
    ) {
        self.mistral_models = mistral_models;
        self.ai21_models = ai21_models;
    }

    pub(super) fn set_json(&mut self, json: Vec<(ProviderId, JsonProviderLookup)>) {
        self.json = json;
    }
}

pub(super) fn make_world(chat_models: &[&'static str], text_models: &[&'static str]) -> World {
    World {
        probe: Probe {
            chat_models: chat_models.to_vec(),
            text_models: text_models.to_vec(),
        },
        mistral_models: Vec::new(),
        ai21_models: Vec::new(),
        json: Vec::new(),
    }
}

fn selection_of(case: &Case) -> ConfigSelection<ChatConfigTag> {
    let construction = match case.arguments {
        Arguments::Shared => ConfigConstruction::Shared,
        Arguments::None => ConfigConstruction::New(Vec::new()),
        Arguments::Model => ConfigConstruction::New(vec![case.model.to_owned()]),
    };
    ConfigSelection {
        tag: case.tag,
        construction,
    }
}

pub(super) fn check(world: &World, cases: &[Case]) {
    for case in cases {
        let spec = CallSpec {
            provider: case.provider,
            model: case.model,
            base_model: case.base_model,
            bedrock: DEFAULT_BEDROCK,
        };
        assert_eq!(
            world.select(&spec).unwrap(),
            Some(selection_of(case)),
            "{} {case:?}",
            case.model
        );
    }
}

pub(super) fn check_classes(world: &World, cases: &[(ProviderId, &'static str, &'static str)]) {
    let classes: Vec<&str> = cases
        .iter()
        .map(|&(provider, model, _)| {
            world
                .select(&CallSpec::plain(provider, model))
                .unwrap()
                .unwrap()
                .tag
                .class_name()
        })
        .collect();
    let expected: Vec<&str> = cases.iter().map(|&(_, _, class)| class).collect();
    assert_eq!(classes, expected);
}
