import re

import pytest
from litellm.types.llms import bedrock as litellm_bedrock
from litellm.types.llms import bedrock_agentcore as litellm_agentcore
from litellm.types.llms import bedrock_invoke_agents as litellm_invoke_agents
from pydantic import ValidationError

from betterlite.types.llms import bedrock_agentcore, bedrock_invoke_agents
from betterlite.types.llms.bedrock.invoke import (
    batches,
    embeddings,
    images,
    invoke_model,
    marengo,
    nova_embeddings,
    rerank,
)

MODULE_PATH = re.compile(r"(?:[A-Za-z_][A-Za-z0-9_]*\.)+")
LOOSE_TYPES: dict[str, str] = {
    "<class 'object'>": "JsonValue",
    "<class 'list'>": "list[JsonValue]",
    "<class 'dict'>": "dict[str, JsonValue]",
    "Any": "JsonValue",
    "ToolSpecBlock": "dict[str, JsonValue]",
    "SystemToolBlock": "dict[str, JsonValue]",
    "CachePointBlock": "dict[str, JsonValue]",
}


def normalized(text: str) -> str:
    stripped = MODULE_PATH.sub("", text)
    for loose, ported in LOOSE_TYPES.items():
        stripped = stripped.replace(loose, ported)
    return stripped


def test_names() -> None:
    emb = embeddings
    mar = marengo
    nov = nova_embeddings
    img = images
    rr = rerank
    im = invoke_model
    bat = batches
    ac = bedrock_agentcore
    ia = bedrock_invoke_agents
    ref = litellm_bedrock
    ref_ac = litellm_agentcore
    ref_ia = litellm_invoke_agents
    pairs = (
        (emb.CohereEmbeddingRequest, ref.CohereEmbeddingRequest),
        (emb.CohereEmbeddingRequestWithModel, ref.CohereEmbeddingRequestWithModel),
        (emb.CohereEmbeddingResponse, ref.CohereEmbeddingResponse),
        (emb.AmazonTitanV2EmbeddingRequest, ref.AmazonTitanV2EmbeddingRequest),
        (emb.AmazonTitanV2EmbeddingsByType, ref.AmazonTitanV2EmbeddingsByType),
        (emb.AmazonTitanV2EmbeddingResponse, ref.AmazonTitanV2EmbeddingResponse),
        (emb.AmazonTitanG1EmbeddingRequest, ref.AmazonTitanG1EmbeddingRequest),
        (emb.AmazonTitanG1EmbeddingResponse, ref.AmazonTitanG1EmbeddingResponse),
        (emb.AmazonTitanMultimodalEmbeddingConfig, ref.AmazonTitanMultimodalEmbeddingConfig),
        (emb.AmazonTitanMultimodalEmbeddingRequest, ref.AmazonTitanMultimodalEmbeddingRequest),
        (emb.AmazonTitanMultimodalEmbeddingResponse, ref.AmazonTitanMultimodalEmbeddingResponse),
        (mar.TwelveLabsS3Location, ref.TwelveLabsS3Location),
        (mar.TwelveLabsMediaSource, ref.TwelveLabsMediaSource),
        (mar.TwelveLabsMarengoEmbeddingRequest, ref.TwelveLabsMarengoEmbeddingRequest),
        (mar.TwelveLabsMarengoEmbeddingResponse, ref.TwelveLabsMarengoEmbeddingResponse),
        (
            mar.TwelveLabsMarengo3FixedSegmentationConfig,
            ref.TwelveLabsMarengo3FixedSegmentationConfig,
        ),
        (mar.TwelveLabsMarengo3FixedSegmentation, ref.TwelveLabsMarengo3FixedSegmentation),
        (
            mar.TwelveLabsMarengo3DynamicSegmentationConfig,
            ref.TwelveLabsMarengo3DynamicSegmentationConfig,
        ),
        (mar.TwelveLabsMarengo3DynamicSegmentation, ref.TwelveLabsMarengo3DynamicSegmentation),
        (mar.TwelveLabsMarengo3TextInput, ref.TwelveLabsMarengo3TextInput),
        (mar.TwelveLabsMarengo3ImageInput, ref.TwelveLabsMarengo3ImageInput),
        (mar.TwelveLabsMarengo3TimedMediaOptions, ref.TwelveLabsMarengo3TimedMediaOptions),
        (mar.TwelveLabsMarengo3TimedMediaInput, ref.TwelveLabsMarengo3TimedMediaInput),
        (mar.TwelveLabsMarengo3TextImageInput, ref.TwelveLabsMarengo3TextImageInput),
        (mar.TwelveLabsMarengo3NamedMediaSource, ref.TwelveLabsMarengo3NamedMediaSource),
        (mar.TwelveLabsMarengo3MultiInput, ref.TwelveLabsMarengo3MultiInput),
        (mar.TwelveLabsMarengo3RequestBase, ref.TwelveLabsMarengo3RequestBase),
        (mar.TwelveLabsMarengo3TextRequest, ref.TwelveLabsMarengo3TextRequest),
        (mar.TwelveLabsMarengo3ImageRequest, ref.TwelveLabsMarengo3ImageRequest),
        (mar.TwelveLabsMarengo3VideoRequest, ref.TwelveLabsMarengo3VideoRequest),
        (mar.TwelveLabsMarengo3AudioRequest, ref.TwelveLabsMarengo3AudioRequest),
        (mar.TwelveLabsMarengo3TextImageRequest, ref.TwelveLabsMarengo3TextImageRequest),
        (mar.TwelveLabsMarengo3MultiInputRequest, ref.TwelveLabsMarengo3MultiInputRequest),
        (mar.TwelveLabsS3OutputDataConfig, ref.TwelveLabsS3OutputDataConfig),
        (mar.TwelveLabsOutputDataConfig, ref.TwelveLabsOutputDataConfig),
        (mar.TwelveLabsAsyncInvokeRequest, ref.TwelveLabsAsyncInvokeRequest),
        (mar.TwelveLabsAsyncInvokeStatusResponse, ref.TwelveLabsAsyncInvokeStatusResponse),
        (nov.NovaSourceS3Location, ref.NovaSourceS3Location),
        (nov.NovaSourceObject, ref.NovaSourceObject),
        (nov.NovaTextParams, ref.NovaTextParams),
        (nov.NovaImageParams, ref.NovaImageParams),
        (nov.NovaVideoParams, ref.NovaVideoParams),
        (nov.NovaAudioParams, ref.NovaAudioParams),
        (nov.NovaTextSegmentationConfig, ref.NovaTextSegmentationConfig),
        (nov.NovaMediaSegmentationConfig, ref.NovaMediaSegmentationConfig),
        (nov.NovaTextParamsWithSegmentation, ref.NovaTextParamsWithSegmentation),
        (nov.NovaVideoParamsWithSegmentation, ref.NovaVideoParamsWithSegmentation),
        (nov.NovaAudioParamsWithSegmentation, ref.NovaAudioParamsWithSegmentation),
        (nov.NovaSingleEmbeddingParams, ref.NovaSingleEmbeddingParams),
        (nov.NovaSegmentedEmbeddingParams, ref.NovaSegmentedEmbeddingParams),
        (nov.NovaEmbeddingRequest, ref.NovaEmbeddingRequest),
        (nov.NovaEmbeddingItem, ref.NovaEmbeddingItem),
        (nov.NovaEmbeddingResponse, ref.NovaEmbeddingResponse),
        (nov.NovaS3OutputDataConfig, ref.NovaS3OutputDataConfig),
        (nov.NovaOutputDataConfig, ref.NovaOutputDataConfig),
        (nov.NovaAsyncInvokeRequest, ref.NovaAsyncInvokeRequest),
        (img.AmazonStability3TextToImageRequest, ref.AmazonStability3TextToImageRequest),
        (img.AmazonStability3TextToImageResponse, ref.AmazonStability3TextToImageResponse),
        (img.AmazonTitanTextToImageParams, ref.AmazonTitanTextToImageParams),
        (img.AmazonNovaCanvasRequestBase, ref.AmazonNovaCanvasRequestBase),
        (img.AmazonNovaCanvasImageGenerationConfig, ref.AmazonNovaCanvasImageGenerationConfig),
        (img.AmazonNovaCanvasTextToImageParams, ref.AmazonNovaCanvasTextToImageParams),
        (img.AmazonNovaCanvasTextToImageRequest, ref.AmazonNovaCanvasTextToImageRequest),
        (
            img.AmazonNovaCanvasColorGuidedGenerationParams,
            ref.AmazonNovaCanvasColorGuidedGenerationParams,
        ),
        (img.AmazonNovaCanvasColorGuidedRequest, ref.AmazonNovaCanvasColorGuidedRequest),
        (img.AmazonNovaCanvasTextToImageResponse, ref.AmazonNovaCanvasTextToImageResponse),
        (img.AmazonNovaCanvasInpaintingParams, ref.AmazonNovaCanvasInpaintingParams),
        (img.AmazonNovaCanvasInpaintingRequest, ref.AmazonNovaCanvasInpaintingRequest),
        (img.AmazonTitanImageGenerationRequestBody, ref.AmazonTitanImageGenerationRequestBody),
        (img.BedrockPreparedRequest, ref.BedrockPreparedRequest),
        (rr.BedrockRerankTextQuery, ref.BedrockRerankTextQuery),
        (rr.BedrockRerankQuery, ref.BedrockRerankQuery),
        (rr.BedrockRerankModelConfiguration, ref.BedrockRerankModelConfiguration),
        (
            rr.BedrockRerankBedrockRerankingConfiguration,
            ref.BedrockRerankBedrockRerankingConfiguration,
        ),
        (rr.BedrockRerankConfiguration, ref.BedrockRerankConfiguration),
        (rr.BedrockRerankTextDocument, ref.BedrockRerankTextDocument),
        (rr.BedrockRerankInlineDocumentSource, ref.BedrockRerankInlineDocumentSource),
        (rr.BedrockRerankSource, ref.BedrockRerankSource),
        (rr.BedrockRerankRequest, ref.BedrockRerankRequest),
        (im.BedrockToolBlock, ref.BedrockToolBlock),
        (im.AmazonDeepSeekR1StreamingResponse, ref.AmazonDeepSeekR1StreamingResponse),
        (im.BedrockInvokeAnthropicMessagesRequest, ref.BedrockInvokeAnthropicMessagesRequest),
        (bat.BedrockS3InputDataConfig, ref.BedrockS3InputDataConfig),
        (bat.BedrockInputDataConfig, ref.BedrockInputDataConfig),
        (bat.BedrockS3OutputDataConfig, ref.BedrockS3OutputDataConfig),
        (bat.BedrockOutputDataConfig, ref.BedrockOutputDataConfig),
        (bat.BedrockTag, ref.BedrockTag),
        (bat.AwsSessionTag, ref.AwsSessionTag),
        (bat.BedrockCreateBatchRequest, ref.BedrockCreateBatchRequest),
        (bat.BedrockCreateBatchResponse, ref.BedrockCreateBatchResponse),
        (bat.BedrockGetBatchResponse, ref.BedrockGetBatchResponse),
        (ac.AgentCoreRequestPayload, ref_ac.AgentCoreRequestPayload),
        (ac.AgentCoreRequest, ref_ac.AgentCoreRequest),
        (ac.AgentCoreMessageRole, ref_ac.AgentCoreMessageRole),
        (ac.AgentCoreMessageStart, ref_ac.AgentCoreMessageStart),
        (ac.AgentCoreContentBlockDelta, ref_ac.AgentCoreContentBlockDelta),
        (ac.AgentCoreContentBlockDeltaEvent, ref_ac.AgentCoreContentBlockDeltaEvent),
        (ac.AgentCoreContentBlockStop, ref_ac.AgentCoreContentBlockStop),
        (ac.AgentCoreMessageStop, ref_ac.AgentCoreMessageStop),
        (ac.AgentCoreUsage, ref_ac.AgentCoreUsage),
        (ac.AgentCoreMetrics, ref_ac.AgentCoreMetrics),
        (ac.AgentCoreMetadata, ref_ac.AgentCoreMetadata),
        (ac.AgentCoreEventPayload, ref_ac.AgentCoreEventPayload),
        (ac.AgentCoreEvent, ref_ac.AgentCoreEvent),
        (ac.AgentCoreContentBlock, ref_ac.AgentCoreContentBlock),
        (ac.AgentCoreMessage, ref_ac.AgentCoreMessage),
        (ac.AgentCoreFinalMessage, ref_ac.AgentCoreFinalMessage),
        (ac.AgentCoreParsedResponse, ref_ac.AgentCoreParsedResponse),
        (ia.InvokeAgentEventHeaders, ref_ia.InvokeAgentEventHeaders),
        (ia.InvokeAgentUsage, ref_ia.InvokeAgentUsage),
        (ia.InvokeAgentMetadata, ref_ia.InvokeAgentMetadata),
        (ia.InvokeAgentModelInvocationInput, ref_ia.InvokeAgentModelInvocationInput),
        (ia.InvokeAgentModelInvocationOutput, ref_ia.InvokeAgentModelInvocationOutput),
        (ia.InvokeAgentOrchestrationTrace, ref_ia.InvokeAgentOrchestrationTrace),
        (ia.InvokeAgentPreProcessingTrace, ref_ia.InvokeAgentPreProcessingTrace),
        (ia.InvokeAgentTrace, ref_ia.InvokeAgentTrace),
        (ia.InvokeAgentCallerChain, ref_ia.InvokeAgentCallerChain),
        (ia.InvokeAgentTracePayload, ref_ia.InvokeAgentTracePayload),
        (ia.InvokeAgentChunkPayload, ref_ia.InvokeAgentChunkPayload),
        (ia.InvokeAgentEventPayload, ref_ia.InvokeAgentEventPayload),
        (ia.InvokeAgentEvent, ref_ia.InvokeAgentEvent),
    )
    for pair in pairs:
        assert sorted(pair[0].__annotations__) == sorted(pair[1].__annotations__)
        assert pair[0].__required_keys__ == pair[1].__required_keys__
        assert pair[0].__optional_keys__ == pair[1].__optional_keys__
        assert normalized(str(pair[0].__annotations__)) == normalized(str(pair[1].__annotations__))


def test_aws_auth_params_names() -> None:
    ours = batches.AwsAuthParams
    theirs = litellm_bedrock.AwsAuthParams
    assert sorted(ours.__annotations__) == sorted(theirs.__annotations__)
    assert normalized(str(ours.__annotations__)) == normalized(str(theirs.__annotations__))
    assert tuple(ours.model_fields) == tuple(theirs.model_fields)
    assert dict(ours.model_config) == dict(theirs.model_config)
    ours_key = ours.model_fields["aws_access_key_id"]
    theirs_key = theirs.model_fields["aws_access_key_id"]
    assert ours_key.is_required() == theirs_key.is_required()
    request = {"aws_access_key_id": "a", "unexpected": "dropped"}
    reference = theirs.model_validate(request)
    ported = ours.model_validate(request)
    assert ported.model_dump() == reference.model_dump()
    with pytest.raises(ValidationError):
        ported.aws_access_key_id = "b"


def test_aliases() -> None:
    emb = embeddings
    mar = marengo
    nov = nova_embeddings
    bat = batches
    ia = bedrock_invoke_agents
    ref = litellm_bedrock
    ref_ia = litellm_invoke_agents
    pairs = (
        (emb.COHERE_EMBEDDING_INPUT_TYPES, ref.COHERE_EMBEDDING_INPUT_TYPES),
        (mar.TWELVELABS_EMBEDDING_INPUT_TYPES, ref.TWELVELABS_EMBEDDING_INPUT_TYPES),
        (mar.TWELVELABS_EMBEDDING_OPTIONS, ref.TWELVELABS_EMBEDDING_OPTIONS),
        (mar.TWELVELABS_MARENGO_3_INPUT_TYPES, ref.TWELVELABS_MARENGO_3_INPUT_TYPES),
        (mar.TWELVELABS_MARENGO_3_EMBEDDING_OPTIONS, ref.TWELVELABS_MARENGO_3_EMBEDDING_OPTIONS),
        (mar.TWELVELABS_MARENGO_3_EMBEDDING_TYPES, ref.TWELVELABS_MARENGO_3_EMBEDDING_TYPES),
        (mar.TWELVELABS_MARENGO_3_EMBEDDING_SCOPES, ref.TWELVELABS_MARENGO_3_EMBEDDING_SCOPES),
        (mar.TwelveLabsMarengo3Segmentation, ref.TwelveLabsMarengo3Segmentation),
        (mar.TwelveLabsMarengo3EmbeddingRequest, ref.TwelveLabsMarengo3EmbeddingRequest),
        (nov.NOVA_EMBEDDING_PURPOSES, ref.NOVA_EMBEDDING_PURPOSES),
        (nov.NOVA_EMBEDDING_DIMENSIONS, ref.NOVA_EMBEDDING_DIMENSIONS),
        (nov.NOVA_TRUNCATION_MODES, ref.NOVA_TRUNCATION_MODES),
        (nov.NOVA_DETAIL_LEVELS, ref.NOVA_DETAIL_LEVELS),
        (nov.NOVA_EMBEDDING_MODES, ref.NOVA_EMBEDDING_MODES),
        (nov.NOVA_EMBEDDING_TYPES, ref.NOVA_EMBEDDING_TYPES),
        (nov.AmazonEmbeddingRequest, ref.AmazonEmbeddingRequest),
        (bat.BedrockBatchJobStatus, ref.BedrockBatchJobStatus),
        (ia.InvokeAgentEventList, ref_ia.InvokeAgentEventList),
        (ia.InvokeAgentTraceEvent, ref_ia.InvokeAgentTraceEvent),
        (ia.InvokeAgentChunkEvent, ref_ia.InvokeAgentChunkEvent),
    )
    for pair in pairs:
        assert normalized(str(pair[0])) == normalized(str(pair[1]))
    assert batches.AWS_AUTH_PARAM_KEYS == litellm_bedrock.AWS_AUTH_PARAM_KEYS
    assert [repr(member) for member in batches.BedrockBatchRecordKind] == [
        repr(member) for member in litellm_bedrock.BedrockBatchRecordKind
    ]
