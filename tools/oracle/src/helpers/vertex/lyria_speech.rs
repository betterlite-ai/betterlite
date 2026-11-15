use std::collections::BTreeMap;
use std::future::ready;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::BundledCatalog;
use betterlite_vertex::model_policy::lyria::LyriaOverlay;
use betterlite_vertex::studio_interactions::credentials::RequestHeaders;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use betterlite_vertex::vertex_lyria::audio::{LyriaDecode, decode_lyria_audio};
use betterlite_vertex::vertex_lyria::mapping::{
    DropParams, SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_vertex::vertex_lyria::request::{LyriaCall, prepare_request};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod input;
mod tests;

use input::{DecodedCase, Input, MappingCase, PreparedCase};

struct FixedMint;

impl MintAccessToken for FixedMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from("oracle-token"),
            project: project.unwrap_or("project-1").to_owned(),
        })))
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let entries = BTreeMap::new();
        let catalog = BundledCatalog::default().get();
        let overlay = LyriaOverlay {
            runtime: &entries,
            bundled: catalog.as_ref(),
        };
        Ok(match input {
            Input::Mapped { cases } => mapped(&overlay, cases),
            Input::Prepared { cases } => prepared(&overlay, &runtime, cases),
            Input::Decoded { cases } => decoded(&overlay, cases),
        })
    }
}

fn mapped(overlay: &LyriaOverlay<'_>, cases: &[MappingCase]) -> Value {
    let views: Vec<Value> = cases
        .iter()
        .map(|case| mapped_view(overlay, case))
        .collect();
    json!({"cases": views})
}

fn mapped_view(overlay: &LyriaOverlay<'_>, case: &MappingCase) -> Value {
    let drop = DropParams {
        call: case.drop_params,
        global: case.global_drop_params,
    };
    match map_openai_params(
        overlay,
        &case.model,
        &case.params,
        case.voice.as_ref(),
        drop,
    ) {
        Ok(mapping) => json!({
            "supported": SUPPORTED_OPENAI_PARAMS,
            "voice": mapping.voice,
            "params": mapping.params,
        }),
        Err(_error) => json!({"error": true}),
    }
}

fn prepared(overlay: &LyriaOverlay<'_>, runtime: &Runtime, cases: &[PreparedCase]) -> Value {
    let views: Vec<Value> = cases
        .iter()
        .map(|case| prepared_view(overlay, runtime, case))
        .collect();
    json!({"cases": views})
}

fn prepared_view(overlay: &LyriaOverlay<'_>, runtime: &Runtime, case: &PreparedCase) -> Value {
    let drop = DropParams {
        call: case.drop_params,
        global: case.global_drop_params,
    };
    let Ok(mapping) = map_openai_params(
        overlay,
        &case.model,
        &case.params,
        case.voice.as_ref(),
        drop,
    ) else {
        return json!({"error": true});
    };
    let call = LyriaCall {
        model: &case.model,
        api_base: case.api_base.as_deref(),
        project: case.project.as_deref(),
        location: case.location.as_deref(),
        credentials: None,
        input: &case.input,
        optional_params: &mapping.params,
    };
    match runtime.block_on(prepare_request(&FixedMint, overlay, &call, &case.headers)) {
        Ok(request) => json!({
            "url": request.url,
            "body": request.body,
            "headers": header_view(&request.headers),
        }),
        Err(_error) => json!({"error": true}),
    }
}

fn header_view(headers: &RequestHeaders) -> Value {
    let entries: Map<String, Value> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                Value::String(value.expose_secret().to_owned()),
            )
        })
        .collect();
    Value::Object(entries)
}

fn decoded(overlay: &LyriaOverlay<'_>, cases: &[DecodedCase]) -> Value {
    let views: Vec<Value> = cases
        .iter()
        .map(|case| decoded_view(overlay, case))
        .collect();
    json!({"cases": views})
}

fn decoded_view(overlay: &LyriaOverlay<'_>, case: &DecodedCase) -> Value {
    let sniff = |audio: &[u8]| audio.starts_with(b"fLaC").then(|| "audio/flac".to_owned());
    let decode = LyriaDecode {
        model: &case.model,
        status: case.status,
        body: case.body.as_bytes(),
    };
    match decode_lyria_audio(overlay, &decode, &sniff) {
        Ok(audio) => json!({
            "status": audio.status,
            "contentType": audio.content_type,
            "bytes": audio.bytes,
        }),
        Err(_error) => json!({"error": true}),
    }
}
