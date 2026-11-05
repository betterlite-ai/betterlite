use secrecy::{ExposeSecret as _, SecretString};

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::AwsOptionalParams;

fn fill_text(slot: &mut Option<String>, instance: Option<&String>) {
    let missing = slot.as_deref().is_none_or(str::is_empty);
    if let Some(value) = instance.filter(|value| missing && !value.is_empty()) {
        *slot = Some(value.clone());
    }
}

fn fill_secret(slot: &mut Option<SecretString>, instance: Option<&SecretString>) {
    let missing = slot
        .as_ref()
        .is_none_or(|token| token.expose_secret().is_empty());
    if let Some(value) = instance.filter(|value| missing && !value.expose_secret().is_empty()) {
        *slot = Some(value.clone());
    }
}

impl<Resolver> AwsSecretsManager<Resolver> {
    pub(super) fn fill_missing_optional_params(&self, optional_params: &mut AwsOptionalParams) {
        fill_text(
            &mut optional_params.aws_role_name,
            self.aws_role_name.as_ref(),
        );
        fill_text(
            &mut optional_params.aws_session_name,
            self.aws_session_name.as_ref(),
        );
        fill_text(
            &mut optional_params.aws_region_name,
            self.aws_region_name.as_ref(),
        );
        fill_text(
            &mut optional_params.aws_external_id,
            self.aws_external_id.as_ref(),
        );
        fill_text(
            &mut optional_params.aws_profile_name,
            self.aws_profile_name.as_ref(),
        );
        fill_secret(
            &mut optional_params.aws_web_identity_token,
            self.aws_web_identity_token.as_ref(),
        );
        fill_text(
            &mut optional_params.aws_sts_endpoint,
            self.aws_sts_endpoint.as_ref(),
        );
    }
}
