use betterlite_models::identity::provider_id::ProviderId;

use crate::hosted_platform_dispatch::call::EmbeddingCall;
use crate::regional_host_dispatch::embedding::dispatch;
use crate::regional_host_dispatch::error::DispatchError;

use super::support::EmbeddingFixture;

pub(super) fn call_for(
    provider: ProviderId,
    fixture: &EmbeddingFixture,
) -> Result<EmbeddingCall, DispatchError> {
    dispatch(provider, &fixture.context()).expect("the embedding table has this arm")
}

#[test]
fn providers_outside_the_regional_embedding_table_have_no_arm() {
    let fixture = EmbeddingFixture::default();
    let context = fixture.context();
    assert!(dispatch(ProviderId::NlpCloud, &context).is_none());
    assert!(dispatch(ProviderId::Minimax, &context).is_none());
    assert!(dispatch(ProviderId::Modelscope, &context).is_none());
    assert!(dispatch(ProviderId::Tencent, &context).is_none());
    assert!(dispatch(ProviderId::Openai, &context).is_none());
}
