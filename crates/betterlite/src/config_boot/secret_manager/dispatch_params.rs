use betterlite_config::sections::key_management_settings::Settings;
use betterlite_secret_managers::custom_interface::{CustomParams, CustomValue};
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::SecretAccessMode;
use secrecy::ExposeSecret as _;

pub(super) fn dispatch_settings(settings: &Settings) -> SecretManagerSettings {
    SecretManagerSettings {
        primary_secret_name: settings.primary_secret_name.clone(),
        optional_params: custom_optional_params(settings),
    }
}

fn custom_optional_params(settings: &Settings) -> CustomParams {
    let mut params = custom_text_params(settings);
    custom_store_params(&mut params, settings);
    let tags = settings.tags.as_ref().map_or(CustomValue::Null, |tags| {
        CustomValue::Object(
            tags.iter()
                .map(|(name, value)| (name.clone(), CustomValue::String(value.clone())))
                .collect(),
        )
    });
    params.insert("tags".to_owned(), tags);
    let token = settings
        .aws_web_identity_token
        .as_ref()
        .map_or(CustomValue::Null, |token| {
            CustomValue::String(token.expose_secret().to_owned())
        });
    params.insert("aws_web_identity_token".to_owned(), token);
    let regions = settings
        .replica_regions
        .iter()
        .cloned()
        .map(CustomValue::String)
        .collect();
    params.insert("replica_regions".to_owned(), CustomValue::Array(regions));
    params
}

fn custom_text_params(settings: &Settings) -> CustomParams {
    [
        (
            "primary_secret_name",
            settings.primary_secret_name.as_deref(),
        ),
        ("description", settings.description.as_deref()),
        ("kms_key_id", settings.kms_key_id.as_deref()),
        (
            "custom_secret_manager",
            settings.custom_secret_manager.as_deref(),
        ),
        ("aws_region_name", settings.aws_region_name.as_deref()),
        ("aws_role_name", settings.aws_role_name.as_deref()),
        ("aws_session_name", settings.aws_session_name.as_deref()),
        ("aws_external_id", settings.aws_external_id.as_deref()),
        ("aws_profile_name", settings.aws_profile_name.as_deref()),
        ("aws_sts_endpoint", settings.aws_sts_endpoint.as_deref()),
    ]
    .into_iter()
    .map(|(name, text)| {
        let value = text.map_or(CustomValue::Null, |resolved| {
            CustomValue::String(resolved.to_owned())
        });
        (name.to_owned(), value)
    })
    .collect()
}

fn custom_store_params(params: &mut CustomParams, settings: &Settings) {
    let hosted = settings
        .hosted_keys
        .as_ref()
        .map_or(CustomValue::Null, |names| {
            CustomValue::Array(names.iter().cloned().map(CustomValue::String).collect())
        });
    params.insert("hosted_keys".to_owned(), hosted);
    params.insert(
        "store_virtual_keys".to_owned(),
        CustomValue::Bool(settings.store_virtual_keys),
    );
    params.insert(
        "prefix_for_stored_virtual_keys".to_owned(),
        CustomValue::String(settings.prefix_for_stored_virtual_keys.clone()),
    );
    params.insert(
        "access_mode".to_owned(),
        CustomValue::String(access_mode_name(settings.access_mode).to_owned()),
    );
}

pub(super) const fn access_mode_name(mode: SecretAccessMode) -> &'static str {
    match mode {
        SecretAccessMode::ReadOnly => "read_only",
        SecretAccessMode::WriteOnly => "write_only",
        SecretAccessMode::ReadAndWrite => "read_and_write",
    }
}
