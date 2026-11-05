use std::future::Future;
use std::sync::Arc;

pub trait ResponseBody: Send {
    type Error: Send + Sync + 'static;

    fn next_chunk(&mut self) -> impl Future<Output = Option<Result<Vec<u8>, Self::Error>>> + Send;

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn read_all(&mut self) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;
}

pub type ErrorRecorder<Source> = Arc<dyn Fn(&Arc<Source>) + Send + Sync>;

pub struct RecordingBody<Body: ResponseBody> {
    pub inner: Body,
    pub record_error: ErrorRecorder<Body::Error>,
}

impl<Body: ResponseBody> RecordingBody<Body> {
    fn record<Output>(
        &self,
        result: Result<Output, Body::Error>,
    ) -> Result<Output, Arc<Body::Error>> {
        result.map_err(|source| {
            let error = Arc::new(source);
            (self.record_error)(&error);
            error
        })
    }
}

impl<Body: ResponseBody> ResponseBody for RecordingBody<Body> {
    type Error = Arc<Body::Error>;

    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, Self::Error>> {
        let chunk = self.inner.next_chunk().await?;
        Some(self.record(chunk))
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let closed = self.inner.close().await;
        self.record(closed)
    }

    async fn read_all(&mut self) -> Result<Vec<u8>, Self::Error> {
        let content = self.inner.read_all().await;
        self.record(content)
    }
}
