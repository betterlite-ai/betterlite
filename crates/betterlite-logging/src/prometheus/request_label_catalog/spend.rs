use betterlite_prometheus::label_names_catalog::{
    UserApiKeyLabelName as Label, ZERO_COST_REASON_LABEL,
};

pub(super) const SPEND: &[&str] = &[
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
    Label::ServiceTier.as_str(),
];

pub(super) const ZERO_COST_REQUESTS_TOTAL: &[&str] = &[
    Label::RequestedModel.as_str(),
    Label::Model.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
    ZERO_COST_REASON_LABEL,
];

pub(super) const CAPTURE_RATE: &[&str] = &[Label::ApiProvider.as_str()];
