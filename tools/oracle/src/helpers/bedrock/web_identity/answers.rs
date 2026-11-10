use super::fixture::Answer;
use super::input::{ReplyCredentials, WebIdentityReply};

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

fn error_xml(code: &str, message: &str) -> String {
    format!(
        "<ErrorResponse><Error><Type>Sender</Type><Code>{code}</Code><Message>{message}</Message></Error><RequestId>oracle</RequestId></ErrorResponse>"
    )
}

pub(super) fn denied_identity() -> Answer {
    Answer {
        status: 403,
        body: error_xml("AccessDenied", "the fixture identity call is denied"),
    }
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
        WebIdentityReply::InvalidIdentityToken { message } => Answer {
            status: 400,
            body: error_xml("InvalidIdentityToken", message),
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
