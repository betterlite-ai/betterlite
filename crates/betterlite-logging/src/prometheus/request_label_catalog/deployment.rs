use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName as Label;

pub(super) const STATE: &[&str] = &[
    Label::LitellmModelName.as_str(),
    Label::ModelId.as_str(),
    Label::ApiBase.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const TPM_LIMIT: &[&str] = &[
    Label::ModelGroup.as_str(),
    Label::LitellmModelName.as_str(),
    Label::ModelId.as_str(),
    Label::ApiBase.as_str(),
    Label::ApiProvider.as_str(),
];

pub(super) const COOLED_DOWN: &[&str] = &[
    Label::LitellmModelName.as_str(),
    Label::ModelId.as_str(),
    Label::ApiBase.as_str(),
    Label::ApiProvider.as_str(),
    Label::ExceptionStatus.as_str(),
];
