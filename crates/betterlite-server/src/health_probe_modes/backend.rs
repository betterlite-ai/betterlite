use super::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use super::error::HealthProbeError;
use async_trait::async_trait;
use betterlite_ocr::ocr_document::OcrDocument;
// one method per health check call; the implementor turns the arguments into the provider operation
#[async_trait]
pub trait HealthProbeBackend: Send + Sync {
    async fn chat(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn completion(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn embedding(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn audio_speech(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn audio_transcription(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError>;
    async fn image_generation(&self, call: ProbeCall)
    -> Result<HealthProbeReply, HealthProbeError>;
    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn video_generation(&self, call: ProbeCall)
    -> Result<HealthProbeReply, HealthProbeError>;
    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn realtime(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn batch(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn responses(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError>;
    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError>;
}
// pick_cheapest_chat_models_from_llm_provider behind the wildcard probe
pub trait CheapestChatModels: Send + Sync {
    fn cheapest_chat_models(&self, custom_llm_provider: &str, n: usize) -> Vec<String>;
}
// get_health_check_document for the ocr mode
pub trait OcrHealthDocuments: Send + Sync {
    fn health_check_document(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<OcrDocument, HealthProbeError>;
}
