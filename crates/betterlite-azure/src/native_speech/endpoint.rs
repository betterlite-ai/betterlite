use url::Url;

use crate::native_speech::constants::{
    COGNITIVE_SERVICES_DOMAIN, TTS_ENDPOINT_PATH, TTS_SPEECH_DOMAIN,
};
use crate::native_speech::error::NativeSpeechError;
use crate::native_speech::identity::NativeSpeechIdentity;

pub fn get_complete_url(identity: &NativeSpeechIdentity) -> Result<String, NativeSpeechError> {
    let Some(api_base) = identity.api_base.as_deref() else {
        return Err(NativeSpeechError::MissingApiBase);
    };
    let trimmed = api_base.trim_end_matches('/');
    let hostname = Url::parse(trimmed)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_default();
    if hostname == COGNITIVE_SERVICES_DOMAIN
        || hostname.ends_with(&format!(".{COGNITIVE_SERVICES_DOMAIN}"))
    {
        let region = extract_region(&hostname, COGNITIVE_SERVICES_DOMAIN);
        return Ok(build_tts_url(region));
    }
    if (hostname == TTS_SPEECH_DOMAIN || hostname.ends_with(&format!(".{TTS_SPEECH_DOMAIN}")))
        && trimmed.ends_with(TTS_ENDPOINT_PATH)
    {
        return Ok(trimmed.to_owned());
    }
    Ok(format!("{trimmed}{TTS_ENDPOINT_PATH}"))
}

fn extract_region<'host>(hostname: &'host str, domain: &str) -> &'host str {
    let suffix = format!(".{domain}");
    hostname.strip_suffix(&suffix).unwrap_or("")
}

fn build_tts_url(region: &str) -> String {
    if region.is_empty() {
        format!("https://{TTS_SPEECH_DOMAIN}{TTS_ENDPOINT_PATH}")
    } else {
        format!("https://{region}.{TTS_SPEECH_DOMAIN}{TTS_ENDPOINT_PATH}")
    }
}
