#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::{S3Cache, decode_body};

#[test]
fn replays_the_s3_cache_body_corpus() {
    let corpus = include_bytes!("../../fuzz/corpus/s3_cache_body/json_object");
    drop(decode_body(corpus));
}

#[tokio::test]
async fn configured_transport_reads_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    s3_region_name: us-east-1\n    s3_endpoint_url: http://{address}\n    s3_aws_access_key_id: test-key\n    s3_aws_secret_access_key: test-secret\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap();
    let cache = S3Cache::new_async(&config.sections.s3_response_cache).await;
    let server = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        let count = stream.read(&mut request).await.unwrap();
        assert!(count > 0);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
            )
            .await
            .unwrap();
    };
    let operation = async {
        let (value, ()) = tokio::join!(cache.async_get_cache("response:key"), server);
        assert_eq!(value, decode_body(br#"{"ok":true}"#));
    };
    timeout(Duration::from_secs(10), operation).await.unwrap();
}
