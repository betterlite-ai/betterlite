use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use aws_smithy_runtime_api::client::http::{HttpConnector, HttpConnectorFuture};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use serde_json::{Value, json};

const ASSUMED_CREDENTIALS: &str = "<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>ASIATEMPORARY</AccessKeyId><SecretAccessKey>temporarysecret</SecretAccessKey><SessionToken>temporarytoken</SessionToken><Expiration>2999-01-01T00:00:00Z</Expiration></Credentials></AssumeRoleResult></AssumeRoleResponse>";

#[derive(Clone, Debug, Default)]
pub(super) struct Recorder(Arc<Mutex<Vec<Value>>>);

impl Recorder {
    pub(super) fn calls(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug)]
pub(super) struct Sts(pub(super) Recorder);

impl HttpConnector for Sts {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let body = request.body().bytes().unwrap_or_default();
        let form: Vec<(String, String)> = url::form_urlencoded::parse(body)
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        let field = |wanted: &str| {
            form.iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.clone())
        };
        let authorization = request.headers().get("authorization").unwrap_or_default();
        let scope: Vec<&str> = authorization
            .split("Credential=")
            .nth(1)
            .and_then(|credential| credential.split(',').next())
            .map(|scope| scope.split('/').collect())
            .unwrap_or_default();
        self.0
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "region": scope.get(2),
                "access_key": scope.first(),
                "session_token": request.headers().get("x-amz-security-token"),
                "role_arn": field("RoleArn"),
                "session_name": field("RoleSessionName"),
            }));
        let Ok(status) = StatusCode::try_from(200_u16) else {
            let refused = io::Error::other("status 200 is valid");
            return HttpConnectorFuture::ready(Err(ConnectorError::other(Box::new(refused), None)));
        };
        HttpConnectorFuture::ready(Ok(HttpResponse::new(
            status,
            SdkBody::from(ASSUMED_CREDENTIALS),
        )))
    }
}
