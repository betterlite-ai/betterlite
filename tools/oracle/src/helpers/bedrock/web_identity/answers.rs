use betterlite_bedrock::aws_settings::AwsSessionTag;

use super::fixture::Answer;
use super::input::{IdentityReply, ReplyCredentials, ReplyTag, WebIdentityReply};

pub(super) fn credentials_xml(credentials: &ReplyCredentials) -> String {
    format!(
        "<Credentials><AccessKeyId>{}</AccessKeyId><SecretAccessKey>{}</SecretAccessKey><SessionToken>{}</SessionToken><Expiration>{}</Expiration></Credentials>",
        credentials.access_key_id,
        credentials.secret_access_key,
        credentials.session_token,
        credentials.expiration
    )
}

pub(super) fn packed_xml(packed_policy_size: Option<i32>) -> String {
    packed_policy_size
        .map(|size| format!("<PackedPolicySize>{size}</PackedPolicySize>"))
        .unwrap_or_default()
}

pub(super) fn error_xml(code: &str, message: &str) -> String {
    format!(
        "<ErrorResponse><Error><Type>Sender</Type><Code>{code}</Code><Message>{message}</Message></Error><RequestId>oracle</RequestId></ErrorResponse>"
    )
}

pub(super) fn web_identity_answer(reply: &WebIdentityReply) -> Answer {
    match reply {
        WebIdentityReply::Credentials(credentials) => Answer {
            status: 200,
            body: format!(
                "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                credentials_xml(credentials),
                packed_xml(credentials.packed_policy_size)
            ),
        },
        WebIdentityReply::MissingPackedPolicySize(credentials) => Answer {
            status: 200,
            body: format!(
                "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                credentials_xml(credentials)
            ),
        },
        WebIdentityReply::MissingCredentials => Answer {
            status: 200,
            body: "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult><PackedPolicySize>1</PackedPolicySize></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>".to_owned(),
        },
        WebIdentityReply::InvalidIdentityToken { message } => Answer {
            status: 400,
            body: error_xml("InvalidIdentityToken", message),
        },
        WebIdentityReply::AccessDenied { message } => Answer {
            status: 403,
            body: error_xml("AccessDenied", message),
        },
    }
}

pub(super) fn assume_role_answer(credentials: &ReplyCredentials) -> Answer {
    Answer {
        status: 200,
        body: format!(
            "<AssumeRoleResponse><AssumeRoleResult>{}{}</AssumeRoleResult></AssumeRoleResponse>",
            credentials_xml(credentials),
            packed_xml(credentials.packed_policy_size)
        ),
    }
}

pub(super) fn identity_answer(reply: &IdentityReply) -> Answer {
    match reply {
        IdentityReply::Arn(arn) => Answer {
            status: 200,
            body: format!(
                "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>{arn}</Arn><UserId>AROAORACLE:session</UserId><Account>123456789012</Account></GetCallerIdentityResult></GetCallerIdentityResponse>"
            ),
        },
        IdentityReply::Denied(message) => Answer {
            status: 403,
            body: error_xml("AccessDenied", message),
        },
    }
}

pub(super) fn tags_of(tags: &[ReplyTag]) -> Vec<AwsSessionTag> {
    tags.iter()
        .map(|tag| AwsSessionTag {
            key: tag.key.clone(),
            value: tag.value.clone(),
        })
        .collect()
}
