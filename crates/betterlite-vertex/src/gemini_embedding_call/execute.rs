use std::time::Duration;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::endpoint_target::target::{
    EndpointTarget, TargetProvider, TokenUrlInput, get_token_and_url,
};
use crate::gemini_embedding_records::input::ResolvedFiles;
use crate::gemini_embedding_records::request::{transform_batch_request, transform_embed_content};
use crate::gemini_embedding_records::response::{
    EmbeddingReply, process_batch_reply, process_embed_content_reply,
};
use crate::gemini_embedding_records::usage::TextTokenCounter;
use crate::model_urls::{ApiVersion, UrlMode};
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::MintAccessToken;

use super::call::EmbeddingCall;
use super::error::GeminiEmbeddingCallError;
use super::http::{decode_embedding_http, file_metadata};
use super::plan::{ClientSource, Facade, SUPPLIED_CLIENT_DEADLINE, plan_call};
use super::transport::{EmbeddingTransport, embedding_request, file_request};

const OK: u16 = 200;

// httpx decodes a reply body with errors="replace": one U+FFFD per invalid sequence
fn body_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

pub struct Clients<'client, Transport> {
    pub supplied: Option<&'client Transport>,
    pub pooled: &'client Transport,
}

pub struct Dependencies<'dependency, Transport> {
    pub catalog: &'dependency ModelCatalog,
    pub mint: &'dependency dyn MintAccessToken,
    pub counter: &'dependency (dyn TextTokenCounter + Sync),
    pub clients: Clients<'dependency, Transport>,
}

// Studio needs no token, and its project stays empty because the Studio URL never reads it
async fn authenticate(
    call: &EmbeddingCall<'_>,
    mint: &dyn MintAccessToken,
) -> Result<Option<AccessToken>, GeminiEmbeddingCallError> {
    match call.provider {
        TargetProvider::Gemini => Ok(None),
        TargetProvider::Vertex => mint
            .mint(call.credentials, call.vertex_project)
            .await
            .map(Some)
            .map_err(GeminiEmbeddingCallError::Mint),
    }
}

fn select_target(
    call: &EmbeddingCall<'_>,
    access: Option<&AccessToken>,
    catalog: &ModelCatalog,
) -> Result<EndpointTarget, GeminiEmbeddingCallError> {
    let mode = match call.provider {
        TargetProvider::Vertex => UrlMode::Embedding,
        TargetProvider::Gemini => UrlMode::BatchEmbedding,
    };
    Ok(get_token_and_url(
        catalog,
        &TokenUrlInput {
            provider: call.provider,
            model: call.model,
            auth_token: access.map(|minted| &minted.token),
            gemini_api_key: call.api_key,
            project: access.map(|minted| minted.project.as_str()),
            location: call.vertex_location,
            stream: false,
            api_base: call.api_base,
            api_version: ApiVersion::V1,
            mode,
            use_psc_endpoint_format: false,
        },
    )?)
}

// Each reference is fetched once per occurrence and a later answer replaces an earlier one
async fn resolve_files<Transport>(
    transport: &Transport,
    files: &[String],
    api_key: &SecretString,
    deadline: Duration,
) -> Result<ResolvedFiles, GeminiEmbeddingCallError>
where
    Transport: EmbeddingTransport,
{
    let mut resolved = ResolvedFiles::new();
    for file in files {
        let request = file_request(file, api_key, deadline)?;
        let url = request.url.to_string();
        let reply = transport
            .send(request)
            .await
            .map_err(|source| GeminiEmbeddingCallError::Transport { url, source })?;
        if reply.status != OK {
            return Err(GeminiEmbeddingCallError::FileStatus {
                file: file.clone(),
                status: reply.status,
                body: body_text(&reply.body),
            });
        }
        let metadata = file_metadata(file, &decode_embedding_http(&reply.body)?)?;
        resolved.insert(file.clone(), metadata);
    }
    Ok(resolved)
}

fn request_body(
    call: &EmbeddingCall<'_>,
    facade: Facade,
    resolved: &ResolvedFiles,
) -> Result<Vec<u8>, GeminiEmbeddingCallError> {
    let encoded = match facade {
        Facade::EmbedContent => serde_json::to_vec(&transform_embed_content(
            call.input,
            call.optional_params,
            resolved,
        )?),
        Facade::BatchEmbedContents => serde_json::to_vec(&transform_batch_request(
            call.input,
            call.model,
            call.optional_params,
            resolved,
        )?),
    };
    encoded.map_err(GeminiEmbeddingCallError::Encode)
}

fn process_reply(
    call: &EmbeddingCall<'_>,
    facade: Facade,
    reply: &Json,
    resolved: &ResolvedFiles,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingReply, GeminiEmbeddingCallError> {
    Ok(match facade {
        Facade::EmbedContent => {
            process_embed_content_reply(call.input, call.model, reply, resolved, counter)?
        }
        Facade::BatchEmbedContents => process_batch_reply(call.input, call.model, reply, counter)?,
    })
}

async fn post_embedding<Transport>(
    transport: &Transport,
    request: OutboundRequest,
) -> Result<Json, GeminiEmbeddingCallError>
where
    Transport: EmbeddingTransport,
{
    let url = request.url.to_string();
    let reply = transport
        .send(request)
        .await
        .map_err(|source| GeminiEmbeddingCallError::Transport { url, source })?;
    if reply.status != OK {
        return Err(GeminiEmbeddingCallError::PostStatus {
            status: reply.status,
            body: body_text(&reply.body),
        });
    }
    decode_embedding_http(&reply.body)
}

// Vertex uses embedContent and Studio batchEmbedContents; both authenticate, pick a target, resolve files, post and project
pub async fn gemini_embedding_call<Transport>(
    call: &EmbeddingCall<'_>,
    dependencies: &Dependencies<'_, Transport>,
) -> Result<EmbeddingReply, GeminiEmbeddingCallError>
where
    Transport: EmbeddingTransport,
{
    let access = authenticate(call, dependencies.mint).await?;
    let target = select_target(call, access.as_ref(), dependencies.catalog)?;
    let plan = plan_call(call, dependencies.clients.supplied.is_some())?;
    let clients = &dependencies.clients;
    let transport = match plan.client {
        ClientSource::Supplied => clients.supplied.unwrap_or(clients.pooled),
        ClientSource::Created => clients.pooled,
    };
    let deadline = plan
        .timeouts
        .map_or(SUPPLIED_CLIENT_DEADLINE, |timeouts| timeouts.read);
    let resolved = match call.api_key {
        Some(key) => resolve_files(transport, &plan.files, key, deadline).await?,
        None => ResolvedFiles::new(),
    };
    let body = request_body(call, plan.facade, &resolved)?;
    let request = embedding_request(&target, call.extra_headers, body, deadline)?;
    let decoded = post_embedding(transport, request).await?;
    process_reply(call, plan.facade, &decoded, &resolved, dependencies.counter)
}
