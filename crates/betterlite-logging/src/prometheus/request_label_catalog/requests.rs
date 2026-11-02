use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName as Label;

pub(super) const PROXY_TOTAL: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::RequestedModel.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::StatusCode.as_str(),
    Label::UserEmail.as_str(),
    Label::Route.as_str(),
    Label::ClientIp.as_str(),
    Label::UserAgent.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const PROXY_FAILED: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::RequestedModel.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::UserEmail.as_str(),
    Label::ExceptionStatus.as_str(),
    Label::ExceptionClass.as_str(),
    Label::Route.as_str(),
    Label::ClientIp.as_str(),
    Label::UserAgent.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const REMAINING_REQUESTS: &[&str] = &[
    Label::ModelGroup.as_str(),
    Label::ApiProvider.as_str(),
    Label::ApiBase.as_str(),
    Label::LitellmModelName.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::ModelId.as_str(),
];

pub(super) const REMAINING_TOKENS: &[&str] = &[
    Label::ModelGroup.as_str(),
    Label::ApiProvider.as_str(),
    Label::ApiBase.as_str(),
    Label::LitellmModelName.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::ModelId.as_str(),
];

pub(super) const REQUESTS: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Model.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::UserEmail.as_str(),
    Label::ClientIp.as_str(),
    Label::UserAgent.as_str(),
    Label::RequestedModel.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];
