#![cfg(test)]

use crate::roi_estimator::Estimator;

use super::support::{
    ModelSupport, OK_CONTENT, caller, ok_response, pull, recorded_request, settings,
};

#[tokio::test]
async fn estimate_request_metadata_holds_the_exact_litellm_literals() {
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    estimator
        .estimate(&pull(), &mut ModelSupport::none())
        .await
        .unwrap();
    let metadata = recorded_request(&requests).metadata;
    assert_eq!(metadata.tags, vec!["litellm-roi-estimator".to_owned()]);
    assert!(metadata.litellm_roi_estimator);
}
