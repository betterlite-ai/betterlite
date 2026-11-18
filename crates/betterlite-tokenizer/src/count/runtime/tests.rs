#![cfg(test)]

use std::sync::Arc;

use crate::engine::Engine;
use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::limits::BlockingCounter as _;
use crate::engine::tiktoken::{Encoding, TiktokenEngine};

use super::InlineCounter;

#[tokio::test]
async fn inline_jobs_preserve_results() {
    assert_eq!(InlineCounter.count(Box::new(|| Ok(7))).await.unwrap(), 7);
    assert_eq!(
        InlineCounter
            .convert(Box::new(|| Ok(vec![3, 4])))
            .await
            .unwrap(),
        vec![3, 4]
    );
    let vocabulary = assets::bundled(Encoding::Cl100kBase).unwrap();
    let tokenizer =
        Arc::new(TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary).unwrap());
    let expected = Arc::clone(&tokenizer);
    let built = InlineCounter
        .build(Box::new(move || Ok(Engine::Tiktoken(tokenizer))))
        .await
        .unwrap();
    let Engine::Tiktoken(actual) = built else {
        panic!("the build result must preserve the supplied engine");
    };
    assert!(Arc::ptr_eq(&actual, &expected));
}

#[tokio::test]
async fn inline_jobs_preserve_failures() {
    assert!(matches!(
        InlineCounter
            .count(Box::new(|| Err(EngineError::PoolClosed)))
            .await,
        Err(EngineError::PoolClosed)
    ));
    assert!(matches!(
        InlineCounter
            .build(Box::new(|| Err(EngineError::PoolClosed)))
            .await,
        Err(EngineError::PoolClosed)
    ));
    assert!(matches!(
        InlineCounter
            .convert(Box::new(|| Err(EngineError::PoolClosed)))
            .await,
        Err(EngineError::PoolClosed)
    ));
}
