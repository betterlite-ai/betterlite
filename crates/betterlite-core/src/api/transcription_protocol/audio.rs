use base64::{
    Engine as _, alphabet,
    engine::{GeneralPurpose, GeneralPurposeConfig},
};
use betterlite_models::decisions::Json;

// ast-grep-ignore: no-long-names
use super::RealtimeTranscriptionProtocolError as ProtocolError;

// binascii's strict mode, which b64decode(validate=True) uses, requires canonical padding and ignores nonzero trailing bits
const STRICT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

pub fn decode_pcm16_append<E>(
    audio: Option<&Json>,
    max_encoded_bytes: Option<usize>,
) -> Result<Vec<u8>, E>
where
    E: From<ProtocolError>,
{
    let Some(Json::Text(text)) = audio else {
        return Err(E::from(ProtocolError::AudioNotString));
    };
    if max_encoded_bytes.is_some_and(|limit| text.chars().count() > limit) {
        return Err(E::from(ProtocolError::AudioBacklog));
    }
    let decoded = STRICT_BASE64
        .decode(text.as_bytes())
        .map_err(|_| ProtocolError::InvalidBase64)?;
    if !decoded.len().is_multiple_of(2) {
        return Err(E::from(ProtocolError::IncompleteSamples));
    }
    Ok(decoded)
}
