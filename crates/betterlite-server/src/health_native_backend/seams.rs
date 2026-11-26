use async_trait::async_trait;

use crate::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
};
use crate::health_probe_modes::error::HealthProbeError;

// embedding has no native owner yet; the later implementation calls litellm.aembedding
#[async_trait]
pub trait EmbeddingProbeCall: Send + Sync {
    async fn embed(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// completion has no native owner yet; the later implementation calls litellm.atext_completion
#[async_trait]
pub trait TextCompletionProbeCall: Send + Sync {
    async fn complete(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B1083's aspeech
#[async_trait]
pub trait SpeechProbeCall: Send + Sync {
    async fn speak(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B1082's atranscription
#[async_trait]
pub trait TranscriptionProbeCall: Send + Sync {
    async fn transcribe(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError>;
}

// B998's aimage_generation
#[async_trait]
pub trait ImageGenerationProbeCall: Send + Sync {
    async fn generate(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B999's aimage_edit
#[async_trait]
pub trait ImageEditProbeCall: Send + Sync {
    async fn edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B1014's arerank
#[async_trait]
pub trait RerankProbeCall: Send + Sync {
    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B1129's list_batches
#[async_trait]
pub trait BatchProbeCall: Send + Sync {
    async fn list_batches(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B932's aresponses
#[async_trait]
pub trait ResponsesProbeCall: Send + Sync {
    async fn create(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B6809's aocr; B8540's OcrProbe is its production implementation
#[async_trait]
pub trait OcrProbeCall: Send + Sync {
    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError>;
}

// B3515's asystemone; EvaluationProbe is its production implementation
#[async_trait]
pub trait EvaluationProbeCall: Send + Sync {
    async fn evaluate(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError>;
}
