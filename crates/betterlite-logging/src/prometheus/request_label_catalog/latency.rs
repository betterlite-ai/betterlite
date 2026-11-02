use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName as Label;

pub(super) const API_LATENCY: &[&str] = &[
    Label::Model.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::RequestedModel.as_str(),
    Label::EndUser.as_str(),
    Label::User.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
    Label::ServiceTier.as_str(),
];

pub(super) const TIME_TO_FIRST_TOKEN: &[&str] = &[
    Label::Model.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::RequestedModel.as_str(),
    Label::EndUser.as_str(),
    Label::User.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
    Label::ServiceTier.as_str(),
];

pub(super) const REQUEST_TOTAL_LATENCY: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::RequestedModel.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::Model.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
    Label::ServiceTier.as_str(),
];

pub(super) const REQUEST_QUEUE_TIME: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::RequestedModel.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::Model.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const PER_OUTPUT_TOKEN: &[&str] = &[
    Label::LitellmModelName.as_str(),
    Label::ModelId.as_str(),
    Label::ApiBase.as_str(),
    Label::ApiProvider.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
];

pub(super) const OVERHEAD: &[&str] = &[
    Label::ModelGroup.as_str(),
    Label::ApiProvider.as_str(),
    Label::ApiBase.as_str(),
    Label::LitellmModelName.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::ModelId.as_str(),
];

pub(super) const OVERHEAD_WITH_GUARDRAILS: &[&str] = &[
    Label::ModelGroup.as_str(),
    Label::ApiProvider.as_str(),
    Label::ApiBase.as_str(),
    Label::LitellmModelName.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::ModelId.as_str(),
];
