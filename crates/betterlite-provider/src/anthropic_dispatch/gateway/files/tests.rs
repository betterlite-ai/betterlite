#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

use super::{Files, Prefetched, prefetch};

const ONE: &str = "http://files.example/one.pdf";
const TWO: &str = "http://files.example/two.pdf";

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<String>>,
}

impl RemoteMediaFetcher for Recorder {
    fn convert_url(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<String, ImageFetchError>> + Send {
        self.calls
            .lock()
            .expect("the call log")
            .push(url.to_owned());
        async move { Ok(format!("data:application/pdf;base64,{url}")) }
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> + Send {
        let requested = urls.to_vec();
        async move {
            let mut converted = Vec::with_capacity(requested.len());
            for url in requested {
                converted.push(self.convert_url(&url).await?);
            }
            Ok(converted)
        }
    }
}

fn file_message(url: &str) -> Json {
    let source = format!(
        r#"{{"role":"user","content":[{{"type":"file","file":{{"file_data":"{url}","format":"application/pdf"}}}}]}}"#
    );
    serde_json::from_str(&source).expect("the file message is a JSON object")
}

// Each distinct URL is fetched once, in message order, however often the messages name it.
#[tokio::test]
async fn the_prefetch_fetches_each_file_url_once() {
    let recorder = Recorder::default();
    let messages = vec![file_message(ONE), file_message(TWO), file_message(ONE)];
    let fetched = prefetch(&recorder, &messages).await;
    let keys: Vec<String> = fetched.keys().cloned().collect();
    assert_eq!(keys, vec![String::from(ONE), String::from(TWO)]);
    let calls = recorder.calls.lock().expect("the call log").clone();
    assert_eq!(calls, vec![String::from(ONE), String::from(TWO)]);
    assert!(fetched.values().all(Result::is_ok));
}

// A batch lookup answers from the prefetch and only fetches the URLs it does not hold.
#[tokio::test]
async fn the_files_convert_urls_resolve_through_the_prefetch() {
    let recorder = Recorder::default();
    let prefetched = Prefetched::from([(
        String::from(ONE),
        Ok(String::from("data:application/pdf;base64,prefetched")),
    )]);
    let files = Files {
        fetcher: &recorder,
        fetched: &prefetched,
    };
    let urls = vec![String::from(ONE), String::from(TWO)];
    let converted = files.convert_urls(&urls).await.expect("both urls convert");
    assert_eq!(
        converted,
        vec![
            String::from("data:application/pdf;base64,prefetched"),
            format!("data:application/pdf;base64,{TWO}"),
        ]
    );
    let calls = recorder.calls.lock().expect("the call log").clone();
    assert_eq!(calls, vec![String::from(TWO)]);
}
