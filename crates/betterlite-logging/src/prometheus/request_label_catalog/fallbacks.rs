use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName as Label;

pub(super) const SUCCESSFUL: &[&str] = &[
    Label::RequestedModel.as_str(),
    Label::FallbackModel.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::ExceptionStatus.as_str(),
    Label::ExceptionClass.as_str(),
    Label::ModelId.as_str(),
];
