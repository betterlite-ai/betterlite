use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use aws_smithy_async::{
    rt::sleep::{SharedAsyncSleep, TokioSleep},
    time::{SharedTimeSource, StaticTimeSource},
};
use aws_smithy_runtime_api::client::{
    http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
    orchestrator::{HttpRequest, HttpResponse},
    result::ConnectorError,
};
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;

use crate::aws_settings::sources::NativeSources;

#[derive(Clone, Copy, Debug)]
pub(super) enum Reply {
    Identity(&'static str),
    Raw(u16, &'static str),
    Refused,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Recorder(Arc<Mutex<Vec<(String, String)>>>);

impl Recorder {
    pub(super) fn count(&self, action: &str) -> usize {
        let calls = self.0.lock().unwrap();
        calls
            .iter()
            .filter(|(_, body)| body.contains(&format!("Action={action}")))
            .count()
    }

    pub(super) fn authorizations(&self) -> Vec<String> {
        let calls = self.0.lock().unwrap();
        calls
            .iter()
            .map(|(authorization, _)| authorization.clone())
            .collect()
    }
}

#[derive(Debug)]
struct Canned {
    reply: Reply,
    recorder: Recorder,
}

impl HttpConnector for Canned {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let body = String::from_utf8(request.body().bytes().unwrap_or_default().to_vec()).unwrap();
        let authorization = request.headers().get("authorization").unwrap_or_default();
        self.recorder
            .0
            .lock()
            .unwrap()
            .push((authorization.to_owned(), body));
        let (status, xml) = match self.reply {
            Reply::Identity(arn) => (
                200,
                format!(
                    "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>{arn}</Arn><Account>1</Account></GetCallerIdentityResult></GetCallerIdentityResponse>"
                ),
            ),
            Reply::Raw(status, xml) => (status, xml.to_owned()),
            Reply::Refused => {
                return HttpConnectorFuture::ready(Err(ConnectorError::other(
                    Box::new(io::Error::other("fixture refused")),
                    None,
                )));
            }
        };
        let response = HttpResponse::new(StatusCode::try_from(status).unwrap(), SdkBody::from(xml));
        HttpConnectorFuture::ready(Ok(response))
    }
}

pub(super) fn sources(environment: &[(&str, &str)], reply: Reply) -> (NativeSources, Recorder) {
    let recorder = Recorder::default();
    let shared = recorder.clone();
    let sources = NativeSources {
        environment: environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        filesystem: Fs::from_slice(&[]),
        client: http_client_fn(move |_, _| {
            SharedHttpConnector::new(Canned {
                reply,
                recorder: shared.clone(),
            })
        }),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    };
    (sources, recorder)
}
