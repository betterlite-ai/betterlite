use betterlite_openai::infinity_embedding;
use betterlite_openai::jina_embedding;
use betterlite_openai::openai_base_config::{Headers, Object as BaseObject};
use secrecy::SecretString;

use super::super::error::FamilyError;
use super::super::request::call;
use super::super::{Call, Decode, JinaEmbeddingDecode};
use super::Leaf;

// the hosted families share one shape, and each step is the one their leaf module offers
pub(super) fn prepare(kind: Kind, leaf: &Leaf<'_, '_>) -> Result<Call, FamilyError> {
    let (name, endpoint, key) = target(kind, leaf)?;
    let supplied: BaseObject = serde_json::from_value(serde_json::to_value(leaf.non_default)?)?;
    let mut mapped = BaseObject::new();
    match kind {
        Kind::Jina => jina_embedding::map_openai_params(&supplied, &mut mapped),
        Kind::Infinity => infinity_embedding::map_openai_params(&supplied, &mut mapped),
    }
    let given = serde_json::to_value(&leaf.request.input)?;
    let (body, headers, decode) = match kind {
        Kind::Jina => {
            let body =
                jina_embedding::transform_embedding_request(leaf.common.model, &given, &mapped);
            let headers = jina_embedding::validate_environment(&Headers::new(), key.as_ref());
            let decode = Decode::JinaEmbedding(Box::new(JinaEmbeddingDecode {
                api_key: key,
                request_data: body.clone(),
            }));
            (body, headers, decode)
        }
        Kind::Infinity => {
            let body =
                infinity_embedding::transform_embedding_request(leaf.common.model, &given, &mapped);
            let headers = infinity_embedding::validate_environment(
                &Headers::new(),
                key.as_ref(),
                leaf.common.host,
            );
            (body, headers, Decode::InfinityEmbedding)
        }
    };
    call(leaf.common, (name, &endpoint), &headers, &body, decode)
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Jina,
    Infinity,
}

// Jina resolves its base and key through its settings; Infinity takes the caller's as given
fn target(
    kind: Kind,
    leaf: &Leaf<'_, '_>,
) -> Result<(&'static str, String, Option<SecretString>), FamilyError> {
    match kind {
        Kind::Jina => {
            let credentials = jina_embedding::resolve_credentials(
                leaf.common.api_base,
                leaf.common.api_key,
                leaf.common.host,
            );
            Ok((
                "jina",
                jina_embedding::get_complete_url(Some(credentials.api_base.as_str())),
                credentials.api_key,
            ))
        }
        Kind::Infinity => Ok((
            "infinity",
            infinity_embedding::get_complete_url(leaf.common.api_base)?,
            leaf.common.api_key.cloned(),
        )),
    }
}
