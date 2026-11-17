use google_cloud_aiplatform_v1::client::VertexRagDataService;
use google_cloud_aiplatform_v1::model::import_rag_files_config::ImportSource;
use google_cloud_aiplatform_v1::model::rag_file_chunking_config::{
    ChunkingConfig, FixedLengthChunking,
};
use google_cloud_aiplatform_v1::model::{
    GcsSource, ImportRagFilesConfig, RagFileChunkingConfig, RagFileTransformationConfig,
};
use google_cloud_auth::credentials::Credentials;
use google_cloud_lro::Poller as _;

use crate::vertex_rag_ingest::ports::{
    CorpusImport, ImportError, ImportFuture, ImportOutcome, ImportRequest,
};

// vertexai.init(project, location) reads application-default credentials; the endpoint and any credentials are injected here
#[derive(Clone, Debug)]
pub enum VertexRagDataImport {
    Adc {
        endpoint: String,
    },
    Credentials {
        endpoint: String,
        credentials: Credentials,
    },
    Existing(VertexRagDataService),
}

impl VertexRagDataImport {
    #[must_use]
    pub fn new(endpoint: &str, credentials: Option<Credentials>) -> Self {
        match credentials {
            Some(credentials) => Self::Credentials {
                endpoint: endpoint.to_owned(),
                credentials,
            },
            None => Self::Adc {
                endpoint: endpoint.to_owned(),
            },
        }
    }

    async fn open(&self) -> Result<VertexRagDataService, ImportError> {
        let builder = match self {
            Self::Existing(client) => return Ok(client.clone()),
            Self::Adc { endpoint } => {
                VertexRagDataService::builder().with_endpoint(endpoint.as_str())
            }
            Self::Credentials {
                endpoint,
                credentials,
            } => VertexRagDataService::builder()
                .with_endpoint(endpoint.as_str())
                .with_credentials(Credentials::clone(credentials)),
        };
        builder
            .build::<VertexRagDataService>()
            .await
            .map_err(|source| ImportError::Client { source })
    }
}

impl CorpusImport for VertexRagDataImport {
    fn import<'request>(&'request self, request: ImportRequest) -> ImportFuture<'request> {
        Box::pin(async move {
            let client = self.open().await?;
            let chunking = request.chunking.chunking_config;
            let fixed = fixed_length_chunking(chunking.chunk_size, chunking.chunk_overlap)?;
            let transformation = RagFileTransformationConfig::new().set_rag_file_chunking_config(
                RagFileChunkingConfig::new().set_chunking_config(Some(
                    ChunkingConfig::FixedLengthChunking(Box::new(fixed)),
                )),
            );
            let config = ImportRagFilesConfig::new()
                .set_rag_file_transformation_config(transformation)
                .set_import_source(ImportSource::GcsSource(Box::new(
                    GcsSource::new().set_uris([request.gcs_uri]),
                )));
            let response = client
                .import_rag_files()
                .set_parent(request.corpus)
                .set_import_rag_files_config(config)
                .poller()
                .until_done()
                .await
                .map_err(|source| ImportError::Rpc { source })?;
            Ok(ImportOutcome {
                imported_rag_files_count: response.imported_rag_files_count,
            })
        })
    }
}

fn fixed_length_chunking(
    chunk_size: i64,
    chunk_overlap: i64,
) -> Result<FixedLengthChunking, ImportError> {
    Ok(FixedLengthChunking::new()
        .set_chunk_size(
            i32::try_from(chunk_size).map_err(|_source| ImportError::IntegerRange {
                field: "chunk_size",
                value: chunk_size,
            })?,
        )
        .set_chunk_overlap(i32::try_from(chunk_overlap).map_err(|_source| {
            ImportError::IntegerRange {
                field: "chunk_overlap",
                value: chunk_overlap,
            }
        })?))
}
