use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName as Label;

pub(super) const INPUT: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Model.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::UserEmail.as_str(),
    Label::RequestedModel.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const TOTAL: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Model.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::UserEmail.as_str(),
    Label::RequestedModel.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const OUTPUT: &[&str] = &[
    Label::EndUser.as_str(),
    Label::HashedApiKey.as_str(),
    Label::ApiKeyAlias.as_str(),
    Label::Model.as_str(),
    Label::Team.as_str(),
    Label::TeamAlias.as_str(),
    Label::User.as_str(),
    Label::UserEmail.as_str(),
    Label::RequestedModel.as_str(),
    Label::ModelId.as_str(),
    Label::ApiProvider.as_str(),
];
