import json
import re
from collections.abc import Sequence

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

# Recorded from LiteLLM 1.105.1 at d665dc1a; litellm/types imports need an ignore the budget bans.
MODULE_PATH = re.compile(r"(?:[A-Za-z_][A-Za-z0-9_]*\.)+")


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
    cases = (
        (emb.CohereEmbeddingRequest.__required_keys__, emb.CohereEmbeddingRequest.__optional_keys__,
         emb.CohereEmbeddingRequest, (
            "required: input_type | optional: embedding_types images output_dimension texts ",
            "truncate | annotations: {'texts': list[str], 'images': list[str], 'input_type': ",
            "Required[Literal['search_document', 'search_query', 'classification', 'clustering', ",
            "'image']], 'truncate': Literal['NONE', 'START', 'END'], 'embedding_types': ",
            "Literal['float', 'int8', 'uint8', 'binary', 'ubinary'], 'output_dimension': <class ",
            "'int'>}",
        )),
        (emb.CohereEmbeddingRequestWithModel.__required_keys__,
         emb.CohereEmbeddingRequestWithModel.__optional_keys__,
         emb.CohereEmbeddingRequestWithModel, (
            "required: input_type model | optional: embedding_types images output_dimension ",
            "texts truncate | annotations: {'texts': list[str], 'images': list[str], ",
            "'input_type': Required[Literal['search_document', 'search_query', 'classification', ",
            "'clustering', 'image']], 'truncate': Literal['NONE', 'START', 'END'], ",
            "'embedding_types': Literal['float', 'int8', 'uint8', 'binary', 'ubinary'], ",
            "'output_dimension': <class 'int'>, 'model': Required[str]}",
        )),
        (emb.CohereEmbeddingResponse.__required_keys__,
         emb.CohereEmbeddingResponse.__optional_keys__,
         emb.CohereEmbeddingResponse, (
            "required: embeddings id response_type texts | optional:  | annotations: ",
            "{'embeddings': list[list[float]], 'id': <class 'str'>, 'response_type': ",
            "Literal['embedding_floats'], 'texts': list[str]}",
        )),
        (emb.AmazonTitanV2EmbeddingRequest.__required_keys__,
         emb.AmazonTitanV2EmbeddingRequest.__optional_keys__,
         emb.AmazonTitanV2EmbeddingRequest, (
            "required: inputText | optional: dimensions embeddingTypes normalize | annotations: ",
            "{'inputText': Required[str], 'dimensions': <class 'int'>, 'normalize': <class ",
            "'bool'>, 'embeddingTypes': list[Literal['float', 'binary']]}",
        )),
        (emb.AmazonTitanV2EmbeddingsByType.__required_keys__,
         emb.AmazonTitanV2EmbeddingsByType.__optional_keys__,
         emb.AmazonTitanV2EmbeddingsByType, (
            "required:  | optional: binary float | annotations: {'binary': list[int], 'float': ",
            "list[float]}",
        )),
        (emb.AmazonTitanV2EmbeddingResponse.__required_keys__,
         emb.AmazonTitanV2EmbeddingResponse.__optional_keys__,
         emb.AmazonTitanV2EmbeddingResponse, (
            "required: inputTextTokenCount | optional: embedding embeddingsByType | annotations: ",
            "{'embedding': list[float], 'embeddingsByType': <class ",
            "'AmazonTitanV2EmbeddingsByType'>, 'inputTextTokenCount': Required[int]}",
        )),
        (emb.AmazonTitanG1EmbeddingRequest.__required_keys__,
         emb.AmazonTitanG1EmbeddingRequest.__optional_keys__,
         emb.AmazonTitanG1EmbeddingRequest, (
            "required: inputText | optional:  | annotations: {'inputText': <class 'str'>}",
        )),
        (emb.AmazonTitanG1EmbeddingResponse.__required_keys__,
         emb.AmazonTitanG1EmbeddingResponse.__optional_keys__,
         emb.AmazonTitanG1EmbeddingResponse, (
            "required: embedding inputTextTokenCount | optional:  | annotations: {'embedding': ",
            "list[float], 'inputTextTokenCount': <class 'int'>}",
        )),
        (emb.AmazonTitanMultimodalEmbeddingConfig.__required_keys__,
         emb.AmazonTitanMultimodalEmbeddingConfig.__optional_keys__,
         emb.AmazonTitanMultimodalEmbeddingConfig, (
            "required: outputEmbeddingLength | optional:  | annotations: ",
            "{'outputEmbeddingLength': Literal[256, 384, 1024]}",
        )),
        (emb.AmazonTitanMultimodalEmbeddingRequest.__required_keys__,
         emb.AmazonTitanMultimodalEmbeddingRequest.__optional_keys__,
         emb.AmazonTitanMultimodalEmbeddingRequest, (
            "required:  | optional: embeddingConfig inputImage inputText | annotations: ",
            "{'inputText': <class 'str'>, 'inputImage': <class 'str'>, 'embeddingConfig': <class ",
            "'AmazonTitanMultimodalEmbeddingConfig'>}",
        )),
        (emb.AmazonTitanMultimodalEmbeddingResponse.__required_keys__,
         emb.AmazonTitanMultimodalEmbeddingResponse.__optional_keys__,
         emb.AmazonTitanMultimodalEmbeddingResponse, (
            "required: embedding inputTextTokenCount message | optional:  | annotations: ",
            "{'embedding': list[float], 'inputTextTokenCount': <class 'int'>, 'message': <class ",
            "'str'>}",
        )),
        (mar.TwelveLabsS3Location.__required_keys__, mar.TwelveLabsS3Location.__optional_keys__,
         mar.TwelveLabsS3Location, (
            "required:  | optional: bucketOwner uri | annotations: {'uri': <class 'str'>, ",
            "'bucketOwner': <class 'str'>}",
        )),
        (mar.TwelveLabsMediaSource.__required_keys__, mar.TwelveLabsMediaSource.__optional_keys__,
         mar.TwelveLabsMediaSource, (
            "required:  | optional: base64String s3Location | annotations: {'base64String': ",
            "<class 'str'>, 's3Location': <class 'TwelveLabsS3Location'>}",
        )),
        (mar.TwelveLabsMarengoEmbeddingRequest.__required_keys__,
         mar.TwelveLabsMarengoEmbeddingRequest.__optional_keys__,
         mar.TwelveLabsMarengoEmbeddingRequest, (
            "required: inputType | optional: embeddingOption inputText lengthSec mediaSource ",
            "minClipSec startSec textTruncate useFixedLengthSec | annotations: {'inputType': ",
            "Required[Literal['text', 'image', 'video', 'audio']], 'inputText': <class 'str'>, ",
            "'mediaSource': <class 'TwelveLabsMediaSource'>, 'textTruncate': Literal['end', ",
            "'none'], 'startSec': <class 'float'>, 'lengthSec': <class 'float'>, ",
            "'useFixedLengthSec': <class 'float'>, 'minClipSec': <class 'int'>, ",
            "'embeddingOption': list[Literal['visual-text', 'visual-image', 'audio']]}",
        )),
        (mar.TwelveLabsMarengoEmbeddingResponse.__required_keys__,
         mar.TwelveLabsMarengoEmbeddingResponse.__optional_keys__,
         mar.TwelveLabsMarengoEmbeddingResponse, (
            "required: embedding embeddingOption endSec startSec | optional:  | annotations: ",
            "{'embedding': list[float], 'embeddingOption': Literal['visual-text', ",
            "'visual-image', 'audio'], 'startSec': <class 'float'>, 'endSec': <class 'float'>}",
        )),
        (mar.TwelveLabsMarengo3FixedSegmentationConfig.__required_keys__,
         mar.TwelveLabsMarengo3FixedSegmentationConfig.__optional_keys__,
         mar.TwelveLabsMarengo3FixedSegmentationConfig, (
            "required: durationSec | optional:  | annotations: {'durationSec': ReadOnly[int]}",
        )),
        (mar.TwelveLabsMarengo3FixedSegmentation.__required_keys__,
         mar.TwelveLabsMarengo3FixedSegmentation.__optional_keys__,
         mar.TwelveLabsMarengo3FixedSegmentation, (
            "required: fixed method | optional:  | annotations: {'method': ",
            "ReadOnly[Literal['fixed']], 'fixed': ",
            "ReadOnly[TwelveLabsMarengo3FixedSegmentationConfig]}",
        )),
        (mar.TwelveLabsMarengo3DynamicSegmentationConfig.__required_keys__,
         mar.TwelveLabsMarengo3DynamicSegmentationConfig.__optional_keys__,
         mar.TwelveLabsMarengo3DynamicSegmentationConfig, (
            "required: minDurationSec | optional:  | annotations: {'minDurationSec': ",
            "ReadOnly[int]}",
        )),
        (mar.TwelveLabsMarengo3DynamicSegmentation.__required_keys__,
         mar.TwelveLabsMarengo3DynamicSegmentation.__optional_keys__,
         mar.TwelveLabsMarengo3DynamicSegmentation, (
            "required: dynamic method | optional:  | annotations: {'method': ",
            "ReadOnly[Literal['dynamic']], 'dynamic': ",
            "ReadOnly[TwelveLabsMarengo3DynamicSegmentationConfig]}",
        )),
        (mar.TwelveLabsMarengo3TextInput.__required_keys__,
         mar.TwelveLabsMarengo3TextInput.__optional_keys__,
         mar.TwelveLabsMarengo3TextInput, (
            "required: inputText | optional:  | annotations: {'inputText': ReadOnly[str]}",
        )),
        (mar.TwelveLabsMarengo3ImageInput.__required_keys__,
         mar.TwelveLabsMarengo3ImageInput.__optional_keys__,
         mar.TwelveLabsMarengo3ImageInput, (
            "required: mediaSource | optional:  | annotations: {'mediaSource': ",
            "ReadOnly[TwelveLabsMediaSource]}",
        )),
        (mar.TwelveLabsMarengo3TimedMediaOptions.__required_keys__,
         mar.TwelveLabsMarengo3TimedMediaOptions.__optional_keys__,
         mar.TwelveLabsMarengo3TimedMediaOptions, (
            "required:  | optional: embeddingOption embeddingScope embeddingType endSec ",
            "segmentation startSec | annotations: {'startSec': ReadOnly[float], 'endSec': ",
            "ReadOnly[float], 'segmentation': ReadOnly[TwelveLabsMarengo3FixedSegmentation | ",
            "TwelveLabsMarengo3DynamicSegmentation], 'embeddingOption': ",
            "ReadOnly[Sequence[Literal['visual', 'audio', 'transcription']]], 'embeddingType': ",
            "ReadOnly[Sequence[Literal['separate_embedding', 'fused_embedding']]], ",
            "'embeddingScope': ReadOnly[Sequence[Literal['clip', 'asset']]]}",
        )),
        (mar.TwelveLabsMarengo3TimedMediaInput.__required_keys__,
         mar.TwelveLabsMarengo3TimedMediaInput.__optional_keys__,
         mar.TwelveLabsMarengo3TimedMediaInput, (
            "required: mediaSource | optional: embeddingOption embeddingScope embeddingType ",
            "endSec segmentation startSec | annotations: {'startSec': ReadOnly[float], 'endSec': ",
            "ReadOnly[float], 'segmentation': ReadOnly[TwelveLabsMarengo3FixedSegmentation | ",
            "TwelveLabsMarengo3DynamicSegmentation], 'embeddingOption': ",
            "ReadOnly[Sequence[Literal['visual', 'audio', 'transcription']]], 'embeddingType': ",
            "ReadOnly[Sequence[Literal['separate_embedding', 'fused_embedding']]], ",
            "'embeddingScope': ReadOnly[Sequence[Literal['clip', 'asset']]], 'mediaSource': ",
            "Required[ReadOnly[TwelveLabsMediaSource]]}",
        )),
        (mar.TwelveLabsMarengo3TextImageInput.__required_keys__,
         mar.TwelveLabsMarengo3TextImageInput.__optional_keys__,
         mar.TwelveLabsMarengo3TextImageInput, (
            "required: inputText mediaSource | optional:  | annotations: {'inputText': ",
            "ReadOnly[str], 'mediaSource': ReadOnly[TwelveLabsMediaSource]}",
        )),
        (mar.TwelveLabsMarengo3NamedMediaSource.__required_keys__,
         mar.TwelveLabsMarengo3NamedMediaSource.__optional_keys__,
         mar.TwelveLabsMarengo3NamedMediaSource, (
            "required: mediaType name | optional: base64String s3Location | annotations: ",
            "{'base64String': <class 'str'>, 's3Location': <class 'TwelveLabsS3Location'>, ",
            "'name': Required[ReadOnly[str]], 'mediaType': Required[ReadOnly[Literal['image']]]}",
        )),
        (mar.TwelveLabsMarengo3MultiInput.__required_keys__,
         mar.TwelveLabsMarengo3MultiInput.__optional_keys__,
         mar.TwelveLabsMarengo3MultiInput, (
            "required: mediaSources | optional: inputText | annotations: {'inputText': ",
            "ReadOnly[str], 'mediaSources': ",
            "Required[ReadOnly[Sequence[TwelveLabsMarengo3NamedMediaSource]]]}",
        )),
        (mar.TwelveLabsMarengo3RequestBase.__required_keys__,
         mar.TwelveLabsMarengo3RequestBase.__optional_keys__,
         mar.TwelveLabsMarengo3RequestBase, (
            "required:  | optional: inferenceId | annotations: {'inferenceId': ReadOnly[str]}",
        )),
        (mar.TwelveLabsMarengo3TextRequest.__required_keys__,
         mar.TwelveLabsMarengo3TextRequest.__optional_keys__,
         mar.TwelveLabsMarengo3TextRequest, (
            "required: inputType text | optional: inferenceId | annotations: {'inferenceId': ",
            "ReadOnly[str], 'inputType': ReadOnly[Literal['text']], 'text': ",
            "ReadOnly[TwelveLabsMarengo3TextInput]}",
        )),
        (mar.TwelveLabsMarengo3ImageRequest.__required_keys__,
         mar.TwelveLabsMarengo3ImageRequest.__optional_keys__,
         mar.TwelveLabsMarengo3ImageRequest, (
            "required: image inputType | optional: inferenceId | annotations: {'inferenceId': ",
            "ReadOnly[str], 'inputType': ReadOnly[Literal['image']], 'image': ",
            "ReadOnly[TwelveLabsMarengo3ImageInput]}",
        )),
        (mar.TwelveLabsMarengo3VideoRequest.__required_keys__,
         mar.TwelveLabsMarengo3VideoRequest.__optional_keys__,
         mar.TwelveLabsMarengo3VideoRequest, (
            "required: inputType video | optional: inferenceId | annotations: {'inferenceId': ",
            "ReadOnly[str], 'inputType': ReadOnly[Literal['video']], 'video': ",
            "ReadOnly[TwelveLabsMarengo3TimedMediaInput]}",
        )),
        (mar.TwelveLabsMarengo3AudioRequest.__required_keys__,
         mar.TwelveLabsMarengo3AudioRequest.__optional_keys__,
         mar.TwelveLabsMarengo3AudioRequest, (
            "required: audio inputType | optional: inferenceId | annotations: {'inferenceId': ",
            "ReadOnly[str], 'inputType': ReadOnly[Literal['audio']], 'audio': ",
            "ReadOnly[TwelveLabsMarengo3TimedMediaInput]}",
        )),
        (mar.TwelveLabsMarengo3TextImageRequest.__required_keys__,
         mar.TwelveLabsMarengo3TextImageRequest.__optional_keys__,
         mar.TwelveLabsMarengo3TextImageRequest, (
            "required: inputType text_image | optional: inferenceId | annotations: ",
            "{'inferenceId': ReadOnly[str], 'inputType': ReadOnly[Literal['text_image']], ",
            "'text_image': ReadOnly[TwelveLabsMarengo3TextImageInput]}",
        )),
        (mar.TwelveLabsMarengo3MultiInputRequest.__required_keys__,
         mar.TwelveLabsMarengo3MultiInputRequest.__optional_keys__,
         mar.TwelveLabsMarengo3MultiInputRequest, (
            "required: inputType multi_input | optional: inferenceId | annotations: ",
            "{'inferenceId': ReadOnly[str], 'inputType': ReadOnly[Literal['multi_input']], ",
            "'multi_input': ReadOnly[TwelveLabsMarengo3MultiInput]}",
        )),
        (mar.TwelveLabsS3OutputDataConfig.__required_keys__,
         mar.TwelveLabsS3OutputDataConfig.__optional_keys__,
         mar.TwelveLabsS3OutputDataConfig, (
            "required: s3Uri | optional:  | annotations: {'s3Uri': <class 'str'>}",
        )),
        (mar.TwelveLabsOutputDataConfig.__required_keys__,
         mar.TwelveLabsOutputDataConfig.__optional_keys__,
         mar.TwelveLabsOutputDataConfig, (
            "required: s3OutputDataConfig | optional:  | annotations: {'s3OutputDataConfig': ",
            "<class 'TwelveLabsS3OutputDataConfig'>}",
        )),
        (mar.TwelveLabsAsyncInvokeRequest.__required_keys__,
         mar.TwelveLabsAsyncInvokeRequest.__optional_keys__,
         mar.TwelveLabsAsyncInvokeRequest, (
            "required: modelId modelInput outputDataConfig | optional:  | annotations: ",
            "{'modelId': <class 'str'>, 'modelInput': ReadOnly[TwelveLabsMarengoEmbeddingRequest ",
            "| TwelveLabsMarengo3TextRequest | TwelveLabsMarengo3ImageRequest | ",
            "TwelveLabsMarengo3VideoRequest | TwelveLabsMarengo3AudioRequest | ",
            "TwelveLabsMarengo3TextImageRequest | TwelveLabsMarengo3MultiInputRequest], ",
            "'outputDataConfig': <class 'TwelveLabsOutputDataConfig'>}",
        )),
        (mar.TwelveLabsAsyncInvokeStatusResponse.__required_keys__,
         mar.TwelveLabsAsyncInvokeStatusResponse.__optional_keys__,
         mar.TwelveLabsAsyncInvokeStatusResponse, (
            "required: clientRequestToken endTime failureMessage invocationArn lastModifiedTime ",
            "modelArn outputDataConfig status submitTime | optional:  | annotations: ",
            "{'invocationArn': <class 'str'>, 'modelArn': <class 'str'>, 'status': <class ",
            "'str'>, 'submitTime': <class 'str'>, 'lastModifiedTime': <class 'str'>, 'endTime': ",
            "str | None, 'outputDataConfig': <class 'TwelveLabsOutputDataConfig'>, ",
            "'clientRequestToken': str | None, 'failureMessage': str | None}",
        )),
        (nov.NovaSourceS3Location.__required_keys__, nov.NovaSourceS3Location.__optional_keys__,
         nov.NovaSourceS3Location, (
            "required: uri | optional:  | annotations: {'uri': <class 'str'>}",
        )),
        (nov.NovaSourceObject.__required_keys__, nov.NovaSourceObject.__optional_keys__,
         nov.NovaSourceObject, (
            "required:  | optional: bytes s3Location | annotations: {'bytes': <class 'str'>, ",
            "'s3Location': <class 'NovaSourceS3Location'>}",
        )),
        (nov.NovaTextParams.__required_keys__, nov.NovaTextParams.__optional_keys__,
         nov.NovaTextParams, (
            "required:  | optional: source truncationMode value | annotations: ",
            "{'truncationMode': Literal['START', 'END', 'NONE'], 'value': <class 'str'>, ",
            "'source': <class 'NovaSourceObject'>}",
        )),
        (nov.NovaImageParams.__required_keys__, nov.NovaImageParams.__optional_keys__,
         nov.NovaImageParams, (
            "required: source | optional: detailLevel format | annotations: {'format': <class ",
            "'str'>, 'source': Required[NovaSourceObject], 'detailLevel': ",
            "Literal['STANDARD_IMAGE', 'DOCUMENT_IMAGE']}",
        )),
        (nov.NovaVideoParams.__required_keys__, nov.NovaVideoParams.__optional_keys__,
         nov.NovaVideoParams, (
            "required: embeddingMode source | optional: format | annotations: {'format': <class ",
            "'str'>, 'source': Required[NovaSourceObject], 'embeddingMode': ",
            "Required[Literal['AUDIO_VIDEO_COMBINED', 'AUDIO_VIDEO_SEPARATE']]}",
        )),
        (nov.NovaAudioParams.__required_keys__, nov.NovaAudioParams.__optional_keys__,
         nov.NovaAudioParams, (
            "required: source | optional: format | annotations: {'format': <class 'str'>, ",
            "'source': Required[NovaSourceObject]}",
        )),
        (nov.NovaTextSegmentationConfig.__required_keys__,
         nov.NovaTextSegmentationConfig.__optional_keys__,
         nov.NovaTextSegmentationConfig, (
            "required:  | optional: maxLengthChars | annotations: {'maxLengthChars': <class ",
            "'int'>}",
        )),
        (nov.NovaMediaSegmentationConfig.__required_keys__,
         nov.NovaMediaSegmentationConfig.__optional_keys__,
         nov.NovaMediaSegmentationConfig, (
            "required:  | optional: durationSeconds | annotations: {'durationSeconds': <class ",
            "'int'>}",
        )),
        (nov.NovaTextParamsWithSegmentation.__required_keys__,
         nov.NovaTextParamsWithSegmentation.__optional_keys__,
         nov.NovaTextParamsWithSegmentation, (
            "required:  | optional: segmentationConfig source truncationMode value | ",
            "annotations: {'truncationMode': Literal['START', 'END', 'NONE'], 'value': <class ",
            "'str'>, 'source': <class 'NovaSourceObject'>, 'segmentationConfig': <class ",
            "'NovaTextSegmentationConfig'>}",
        )),
        (nov.NovaVideoParamsWithSegmentation.__required_keys__,
         nov.NovaVideoParamsWithSegmentation.__optional_keys__,
         nov.NovaVideoParamsWithSegmentation, (
            "required: embeddingMode source | optional: format segmentationConfig | annotations: ",
            "{'format': <class 'str'>, 'source': Required[NovaSourceObject], 'embeddingMode': ",
            "Required[Literal['AUDIO_VIDEO_COMBINED', 'AUDIO_VIDEO_SEPARATE']], ",
            "'segmentationConfig': <class 'NovaMediaSegmentationConfig'>}",
        )),
        (nov.NovaAudioParamsWithSegmentation.__required_keys__,
         nov.NovaAudioParamsWithSegmentation.__optional_keys__,
         nov.NovaAudioParamsWithSegmentation, (
            "required: source | optional: format segmentationConfig | annotations: {'format': ",
            "<class 'str'>, 'source': Required[NovaSourceObject], 'segmentationConfig': <class ",
            "'NovaMediaSegmentationConfig'>}",
        )),
        (nov.NovaSingleEmbeddingParams.__required_keys__,
         nov.NovaSingleEmbeddingParams.__optional_keys__,
         nov.NovaSingleEmbeddingParams, (
            "required: embeddingPurpose | optional: audio embeddingDimension image text video | ",
            "annotations: {'embeddingPurpose': Required[Literal['GENERIC_INDEX', ",
            "'GENERIC_RETRIEVAL', 'TEXT_RETRIEVAL', 'IMAGE_RETRIEVAL', 'VIDEO_RETRIEVAL', ",
            "'DOCUMENT_RETRIEVAL', 'AUDIO_RETRIEVAL', 'CLASSIFICATION', 'CLUSTERING']], ",
            "'embeddingDimension': Literal[256, 384, 1024, 3072], 'text': <class ",
            "'NovaTextParams'>, 'image': <class 'NovaImageParams'>, 'video': <class ",
            "'NovaVideoParams'>, 'audio': <class 'NovaAudioParams'>}",
        )),
        (nov.NovaSegmentedEmbeddingParams.__required_keys__,
         nov.NovaSegmentedEmbeddingParams.__optional_keys__,
         nov.NovaSegmentedEmbeddingParams, (
            "required: embeddingPurpose | optional: audio embeddingDimension image text video | ",
            "annotations: {'embeddingPurpose': Required[Literal['GENERIC_INDEX', ",
            "'GENERIC_RETRIEVAL', 'TEXT_RETRIEVAL', 'IMAGE_RETRIEVAL', 'VIDEO_RETRIEVAL', ",
            "'DOCUMENT_RETRIEVAL', 'AUDIO_RETRIEVAL', 'CLASSIFICATION', 'CLUSTERING']], ",
            "'embeddingDimension': Literal[256, 384, 1024, 3072], 'text': <class ",
            "'NovaTextParamsWithSegmentation'>, 'image': <class 'NovaImageParams'>, 'video': ",
            "<class 'NovaVideoParamsWithSegmentation'>, 'audio': <class ",
            "'NovaAudioParamsWithSegmentation'>}",
        )),
        (nov.NovaEmbeddingRequest.__required_keys__, nov.NovaEmbeddingRequest.__optional_keys__,
         nov.NovaEmbeddingRequest, (
            "required:  | optional: schemaVersion segmentedEmbeddingParams singleEmbeddingParams ",
            "taskType | annotations: {'schemaVersion': <class 'str'>, 'taskType': ",
            "Literal['SINGLE_EMBEDDING', 'SEGMENTED_EMBEDDING'], 'singleEmbeddingParams': <class ",
            "'NovaSingleEmbeddingParams'>, 'segmentedEmbeddingParams': <class ",
            "'NovaSegmentedEmbeddingParams'>}",
        )),
        (nov.NovaEmbeddingItem.__required_keys__, nov.NovaEmbeddingItem.__optional_keys__,
         nov.NovaEmbeddingItem, (
            "required: embedding | optional: embeddingType truncatedCharLength | annotations: ",
            "{'embeddingType': Literal['TEXT', 'IMAGE', 'VIDEO', 'AUDIO', ",
            "'AUDIO_VIDEO_COMBINED'], 'embedding': Required[list[float]], 'truncatedCharLength': ",
            "<class 'int'>}",
        )),
        (nov.NovaEmbeddingResponse.__required_keys__, nov.NovaEmbeddingResponse.__optional_keys__,
         nov.NovaEmbeddingResponse, (
            "required: embeddings | optional:  | annotations: {'embeddings': ",
            "list[NovaEmbeddingItem]}",
        )),
        (nov.NovaS3OutputDataConfig.__required_keys__, nov.NovaS3OutputDataConfig.__optional_keys__,
         nov.NovaS3OutputDataConfig, (
            "required: s3Uri | optional:  | annotations: {'s3Uri': <class 'str'>}",
        )),
        (nov.NovaOutputDataConfig.__required_keys__, nov.NovaOutputDataConfig.__optional_keys__,
         nov.NovaOutputDataConfig, (
            "required: s3OutputDataConfig | optional:  | annotations: {'s3OutputDataConfig': ",
            "<class 'NovaS3OutputDataConfig'>}",
        )),
        (nov.NovaAsyncInvokeRequest.__required_keys__, nov.NovaAsyncInvokeRequest.__optional_keys__,
         nov.NovaAsyncInvokeRequest, (
            "required: modelId modelInput outputDataConfig | optional:  | annotations: ",
            "{'modelId': <class 'str'>, 'modelInput': <class 'NovaEmbeddingRequest'>, ",
            "'outputDataConfig': <class 'NovaOutputDataConfig'>}",
        )),
        (img.AmazonStability3TextToImageRequest.__required_keys__,
         img.AmazonStability3TextToImageRequest.__optional_keys__,
         img.AmazonStability3TextToImageRequest, (
            "required:  | optional: aspect_ratio mode negative_prompt output_format prompt seed ",
            "| annotations: {'prompt': <class 'str'>, 'aspect_ratio': Literal['16:9', '1:1', ",
            "'21:9', '2:3', '3:2', '4:5', '5:4', '9:16', '9:21'], 'mode': ",
            "Literal['image-to-image', 'text-to-image'], 'output_format': Literal['JPEG', ",
            "'PNG'], 'seed': <class 'int'>, 'negative_prompt': <class 'str'>}",
        )),
        (img.AmazonStability3TextToImageResponse.__required_keys__,
         img.AmazonStability3TextToImageResponse.__optional_keys__,
         img.AmazonStability3TextToImageResponse, (
            "required:  | optional: finish_reasons images seeds | annotations: {'images': ",
            "list[str], 'seeds': list[str], 'finish_reasons': list[str]}",
        )),
        (img.AmazonTitanTextToImageParams.__required_keys__,
         img.AmazonTitanTextToImageParams.__optional_keys__,
         img.AmazonTitanTextToImageParams, (
            "required: text | optional: negativeText | annotations: {'text': Required[str], ",
            "'negativeText': <class 'str'>}",
        )),
        (img.AmazonNovaCanvasRequestBase.__required_keys__,
         img.AmazonNovaCanvasRequestBase.__optional_keys__,
         img.AmazonNovaCanvasRequestBase, (
            "required:  | optional:  | annotations: {}",
        )),
        (img.AmazonNovaCanvasImageGenerationConfig.__required_keys__,
         img.AmazonNovaCanvasImageGenerationConfig.__optional_keys__,
         img.AmazonNovaCanvasImageGenerationConfig, (
            "required:  | optional: cfgScale height numberOfImages quality seed width | ",
            "annotations: {'cfgScale': <class 'int'>, 'seed': <class 'int'>, 'quality': ",
            "Literal['standard', 'premium'], 'width': <class 'int'>, 'height': <class 'int'>, ",
            "'numberOfImages': <class 'int'>}",
        )),
        (img.AmazonNovaCanvasTextToImageParams.__required_keys__,
         img.AmazonNovaCanvasTextToImageParams.__optional_keys__,
         img.AmazonNovaCanvasTextToImageParams, (
            "required:  | optional: conditionImage controlMode controlStrength negativeText text ",
            "| annotations: {'text': <class 'str'>, 'negativeText': <class 'str'>, ",
            "'controlStrength': <class 'float'>, 'controlMode': Literal['CANNY_EDIT', ",
            "'SEGMENTATION'], 'conditionImage': <class 'str'>}",
        )),
        (img.AmazonNovaCanvasTextToImageRequest.__required_keys__,
         img.AmazonNovaCanvasTextToImageRequest.__optional_keys__,
         img.AmazonNovaCanvasTextToImageRequest, (
            "required:  | optional: imageGenerationConfig taskType textToImageParams | ",
            "annotations: {'textToImageParams': <class 'AmazonNovaCanvasTextToImageParams'>, ",
            "'taskType': Literal['TEXT_IMAGE'], 'imageGenerationConfig': <class ",
            "'AmazonNovaCanvasImageGenerationConfig'>}",
        )),
        (img.AmazonNovaCanvasColorGuidedGenerationParams.__required_keys__,
         img.AmazonNovaCanvasColorGuidedGenerationParams.__optional_keys__,
         img.AmazonNovaCanvasColorGuidedGenerationParams, (
            "required:  | optional: colors negativeText referenceImage text | annotations: ",
            "{'colors': list[str], 'referenceImage': <class 'str'>, 'text': <class 'str'>, ",
            "'negativeText': <class 'str'>}",
        )),
        (img.AmazonNovaCanvasColorGuidedRequest.__required_keys__,
         img.AmazonNovaCanvasColorGuidedRequest.__optional_keys__,
         img.AmazonNovaCanvasColorGuidedRequest, (
            "required:  | optional: colorGuidedGenerationParams imageGenerationConfig taskType | ",
            "annotations: {'taskType': Literal['COLOR_GUIDED_GENERATION'], ",
            "'colorGuidedGenerationParams': <class ",
            "'AmazonNovaCanvasColorGuidedGenerationParams'>, 'imageGenerationConfig': <class ",
            "'AmazonNovaCanvasImageGenerationConfig'>}",
        )),
        (img.AmazonNovaCanvasTextToImageResponse.__required_keys__,
         img.AmazonNovaCanvasTextToImageResponse.__optional_keys__,
         img.AmazonNovaCanvasTextToImageResponse, (
            "required:  | optional: images | annotations: {'images': list[str]}",
        )),
        (img.AmazonNovaCanvasInpaintingParams.__required_keys__,
         img.AmazonNovaCanvasInpaintingParams.__optional_keys__,
         img.AmazonNovaCanvasInpaintingParams, (
            "required:  | optional: inputImage maskImage negativeText text | annotations: ",
            "{'text': <class 'str'>, 'maskImage': <class 'str'>, 'inputImage': <class 'str'>, ",
            "'negativeText': <class 'str'>}",
        )),
        (img.AmazonNovaCanvasInpaintingRequest.__required_keys__,
         img.AmazonNovaCanvasInpaintingRequest.__optional_keys__,
         img.AmazonNovaCanvasInpaintingRequest, (
            "required:  | optional: imageGenerationConfig inpaintingParams taskType | ",
            "annotations: {'taskType': Literal['INPAINTING'], 'inpaintingParams': <class ",
            "'AmazonNovaCanvasInpaintingParams'>, 'imageGenerationConfig': <class ",
            "'AmazonNovaCanvasImageGenerationConfig'>}",
        )),
        (img.AmazonTitanImageGenerationRequestBody.__required_keys__,
         img.AmazonTitanImageGenerationRequestBody.__optional_keys__,
         img.AmazonTitanImageGenerationRequestBody, (
            "required:  | optional: imageGenerationConfig taskType textToImageParams | ",
            "annotations: {'taskType': Literal['TEXT_IMAGE', 'COLOR_GUIDED_GENERATION', ",
            "'INPAINTING'], 'textToImageParams': <class 'AmazonTitanTextToImageParams'>, ",
            "'imageGenerationConfig': <class 'AmazonNovaCanvasImageGenerationConfig'>}",
        )),
        (img.BedrockPreparedRequest.__required_keys__, img.BedrockPreparedRequest.__optional_keys__,
         img.BedrockPreparedRequest, (
            "required: body data endpoint_url prepped | optional:  | annotations: ",
            "{'endpoint_url': <class 'str'>, 'prepped': JsonValue, 'body': <class 'bytes'>, ",
            "'data': dict[str, JsonValue]}",
        )),
        (rr.BedrockRerankTextQuery.__required_keys__, rr.BedrockRerankTextQuery.__optional_keys__,
         rr.BedrockRerankTextQuery, (
            "required: text | optional:  | annotations: {'text': <class 'str'>}",
        )),
        (rr.BedrockRerankQuery.__required_keys__, rr.BedrockRerankQuery.__optional_keys__,
         rr.BedrockRerankQuery, (
            "required: textQuery type | optional:  | annotations: {'textQuery': <class ",
            "'BedrockRerankTextQuery'>, 'type': Literal['TEXT']}",
        )),
        (rr.BedrockRerankModelConfiguration.__required_keys__,
         rr.BedrockRerankModelConfiguration.__optional_keys__,
         rr.BedrockRerankModelConfiguration, (
            "required: modelArn | optional: modelConfiguration | annotations: {'modelArn': ",
            "Required[str], 'modelConfiguration': dict[str, JsonValue]}",
        )),
        (rr.BedrockRerankBedrockRerankingConfiguration.__required_keys__,
         rr.BedrockRerankBedrockRerankingConfiguration.__optional_keys__,
         rr.BedrockRerankBedrockRerankingConfiguration, (
            "required: modelConfiguration numberOfResults | optional:  | annotations: ",
            "{'modelConfiguration': <class 'BedrockRerankModelConfiguration'>, ",
            "'numberOfResults': <class 'int'>}",
        )),
        (rr.BedrockRerankConfiguration.__required_keys__,
         rr.BedrockRerankConfiguration.__optional_keys__,
         rr.BedrockRerankConfiguration, (
            "required: bedrockRerankingConfiguration type | optional:  | annotations: ",
            "{'bedrockRerankingConfiguration': <class ",
            "'BedrockRerankBedrockRerankingConfiguration'>, 'type': ",
            "Literal['BEDROCK_RERANKING_MODEL']}",
        )),
        (rr.BedrockRerankTextDocument.__required_keys__,
         rr.BedrockRerankTextDocument.__optional_keys__,
         rr.BedrockRerankTextDocument, (
            "required:  | optional: text | annotations: {'text': <class 'str'>}",
        )),
        (rr.BedrockRerankInlineDocumentSource.__required_keys__,
         rr.BedrockRerankInlineDocumentSource.__optional_keys__,
         rr.BedrockRerankInlineDocumentSource, (
            "required:  | optional: jsonDocument textDocument type | annotations: ",
            "{'jsonDocument': dict[str, JsonValue], 'textDocument': <class ",
            "'BedrockRerankTextDocument'>, 'type': Literal['TEXT', 'JSON']}",
        )),
        (rr.BedrockRerankSource.__required_keys__, rr.BedrockRerankSource.__optional_keys__,
         rr.BedrockRerankSource, (
            "required: inlineDocumentSource type | optional:  | annotations: ",
            "{'inlineDocumentSource': <class 'BedrockRerankInlineDocumentSource'>, 'type': ",
            "Literal['INLINE']}",
        )),
        (rr.BedrockRerankRequest.__required_keys__, rr.BedrockRerankRequest.__optional_keys__,
         rr.BedrockRerankRequest, (
            "required: queries rerankingConfiguration sources | optional:  | annotations: ",
            "{'queries': list[BedrockRerankQuery], 'rerankingConfiguration': <class ",
            "'BedrockRerankConfiguration'>, 'sources': list[BedrockRerankSource]}",
        )),
        (im.BedrockToolBlock.__required_keys__, im.BedrockToolBlock.__optional_keys__,
         im.BedrockToolBlock, (
            "required:  | optional: cachePoint systemTool toolSpec | annotations: {'toolSpec': ",
            "dict[str, JsonValue] | None, 'systemTool': dict[str, JsonValue] | None, ",
            "'cachePoint': dict[str, JsonValue] | None}",
        )),
        (im.AmazonDeepSeekR1StreamingResponse.__required_keys__,
         im.AmazonDeepSeekR1StreamingResponse.__optional_keys__,
         im.AmazonDeepSeekR1StreamingResponse, (
            "required: generation generation_token_count prompt_token_count stop_reason | ",
            "optional:  | annotations: {'generation': <class 'str'>, 'generation_token_count': ",
            "<class 'int'>, 'stop_reason': str | None, 'prompt_token_count': <class 'int'>}",
        )),
        (im.BedrockInvokeAnthropicMessagesRequest.__required_keys__,
         im.BedrockInvokeAnthropicMessagesRequest.__optional_keys__,
         im.BedrockInvokeAnthropicMessagesRequest, (
            "required:  | optional: anthropic_beta anthropic_version context_management ",
            "max_tokens messages metadata output_config safeguards stop_sequences system ",
            "temperature thinking tool_choice tools top_k top_p | annotations: ",
            "{'anthropic_version': <class 'str'>, 'max_tokens': <class 'int'>, 'messages': ",
            "list[JsonValue], 'anthropic_beta': list[str], 'system': JsonValue, ",
            "'stop_sequences': list[str], 'temperature': <class 'float'>, 'top_p': <class ",
            "'float'>, 'top_k': <class 'int'>, 'tools': list[JsonValue], 'tool_choice': ",
            "dict[str, JsonValue], 'thinking': dict[str, JsonValue], 'metadata': dict[str, ",
            "JsonValue], 'output_config': dict[str, JsonValue], 'safeguards': list[JsonValue], ",
            "'context_management': dict[str, JsonValue]}",
        )),
        (bat.BedrockS3InputDataConfig.__required_keys__,
         bat.BedrockS3InputDataConfig.__optional_keys__,
         bat.BedrockS3InputDataConfig, (
            "required: s3Uri | optional: s3BucketOwner | annotations: {'s3Uri': <class 'str'>, ",
            "'s3BucketOwner': NotRequired[ReadOnly[str]]}",
        )),
        (bat.BedrockInputDataConfig.__required_keys__, bat.BedrockInputDataConfig.__optional_keys__,
         bat.BedrockInputDataConfig, (
            "required: s3InputDataConfig | optional:  | annotations: {'s3InputDataConfig': ",
            "<class 'BedrockS3InputDataConfig'>}",
        )),
        (bat.BedrockS3OutputDataConfig.__required_keys__,
         bat.BedrockS3OutputDataConfig.__optional_keys__,
         bat.BedrockS3OutputDataConfig, (
            "required:  | optional: s3BucketOwner s3EncryptionKeyId s3Uri | annotations: ",
            "{'s3Uri': <class 'str'>, 's3EncryptionKeyId': str | None, 's3BucketOwner': ",
            "ReadOnly[str]}",
        )),
        (bat.BedrockOutputDataConfig.__required_keys__,
         bat.BedrockOutputDataConfig.__optional_keys__,
         bat.BedrockOutputDataConfig, (
            "required: s3OutputDataConfig | optional:  | annotations: {'s3OutputDataConfig': ",
            "<class 'BedrockS3OutputDataConfig'>}",
        )),
        (bat.BedrockTag.__required_keys__, bat.BedrockTag.__optional_keys__,
         bat.BedrockTag, (
            "required: key value | optional:  | annotations: {'key': <class 'str'>, 'value': ",
            "<class 'str'>}",
        )),
        (bat.AwsSessionTag.__required_keys__, bat.AwsSessionTag.__optional_keys__,
         bat.AwsSessionTag, (
            "required: Key Value | optional:  | annotations: {'Key': <class 'str'>, 'Value': ",
            "<class 'str'>}",
        )),
        (bat.BedrockCreateBatchRequest.__required_keys__,
         bat.BedrockCreateBatchRequest.__optional_keys__,
         bat.BedrockCreateBatchRequest, (
            "required:  | optional: clientRequestToken inputDataConfig jobName modelId ",
            "outputDataConfig roleArn tags timeoutDurationInHours | annotations: {'jobName': ",
            "<class 'str'>, 'roleArn': <class 'str'>, 'modelId': <class 'str'>, ",
            "'inputDataConfig': <class 'BedrockInputDataConfig'>, 'outputDataConfig': <class ",
            "'BedrockOutputDataConfig'>, 'timeoutDurationInHours': int | None, ",
            "'clientRequestToken': str | None, 'tags': list[BedrockTag] | None}",
        )),
        (bat.BedrockCreateBatchResponse.__required_keys__,
         bat.BedrockCreateBatchResponse.__optional_keys__,
         bat.BedrockCreateBatchResponse, (
            "required: jobArn jobName status | optional:  | annotations: {'jobArn': <class ",
            "'str'>, 'jobName': <class 'str'>, 'status': Literal['Submitted', 'InProgress', ",
            "'Completed', 'Failed', 'Stopping', 'Stopped']}",
        )),
        (bat.BedrockGetBatchResponse.__required_keys__,
         bat.BedrockGetBatchResponse.__optional_keys__,
         bat.BedrockGetBatchResponse, (
            "required:  | optional: clientRequestToken endTime inputDataConfig jobArn jobName ",
            "lastModifiedTime message modelId outputDataConfig roleArn status submitTime ",
            "timeoutDurationInHours | annotations: {'jobArn': <class 'str'>, 'jobName': <class ",
            "'str'>, 'modelId': <class 'str'>, 'roleArn': <class 'str'>, 'status': ",
            "Literal['Submitted', 'InProgress', 'Completed', 'Failed', 'Stopping', 'Stopped'], ",
            "'message': str | None, 'submitTime': str | None, 'lastModifiedTime': str | None, ",
            "'endTime': str | None, 'inputDataConfig': <class 'BedrockInputDataConfig'>, ",
            "'outputDataConfig': <class 'BedrockOutputDataConfig'>, 'timeoutDurationInHours': ",
            "int | None, 'clientRequestToken': str | None}",
        )),
        (ac.AgentCoreRequestPayload.__required_keys__, ac.AgentCoreRequestPayload.__optional_keys__,
         ac.AgentCoreRequestPayload, (
            "required: prompt | optional:  | annotations: {'prompt': <class 'str'>}",
        )),
        (ac.AgentCoreRequest.__required_keys__, ac.AgentCoreRequest.__optional_keys__,
         ac.AgentCoreRequest, (
            "required:  | optional: payload | annotations: {'payload': <class 'str'>}",
        )),
        (ac.AgentCoreMessageRole.__required_keys__, ac.AgentCoreMessageRole.__optional_keys__,
         ac.AgentCoreMessageRole, (
            "required: role | optional:  | annotations: {'role': Literal['assistant']}",
        )),
        (ac.AgentCoreMessageStart.__required_keys__, ac.AgentCoreMessageStart.__optional_keys__,
         ac.AgentCoreMessageStart, (
            "required: role | optional:  | annotations: {'role': Literal['assistant']}",
        )),
        (ac.AgentCoreContentBlockDelta.__required_keys__,
         ac.AgentCoreContentBlockDelta.__optional_keys__,
         ac.AgentCoreContentBlockDelta, (
            "required: text | optional:  | annotations: {'text': <class 'str'>}",
        )),
        (ac.AgentCoreContentBlockDeltaEvent.__required_keys__,
         ac.AgentCoreContentBlockDeltaEvent.__optional_keys__,
         ac.AgentCoreContentBlockDeltaEvent, (
            "required: contentBlockIndex delta | optional:  | annotations: {'delta': <class ",
            "'AgentCoreContentBlockDelta'>, 'contentBlockIndex': <class 'int'>}",
        )),
        (ac.AgentCoreContentBlockStop.__required_keys__,
         ac.AgentCoreContentBlockStop.__optional_keys__,
         ac.AgentCoreContentBlockStop, (
            "required: contentBlockIndex | optional:  | annotations: {'contentBlockIndex': ",
            "<class 'int'>}",
        )),
        (ac.AgentCoreMessageStop.__required_keys__, ac.AgentCoreMessageStop.__optional_keys__,
         ac.AgentCoreMessageStop, (
            "required: stopReason | optional:  | annotations: {'stopReason': Literal['end_turn', ",
            "'max_tokens', 'stop_sequence']}",
        )),
        (ac.AgentCoreUsage.__required_keys__, ac.AgentCoreUsage.__optional_keys__,
         ac.AgentCoreUsage, (
            "required: inputTokens outputTokens totalTokens | optional:  | annotations: ",
            "{'inputTokens': <class 'int'>, 'outputTokens': <class 'int'>, 'totalTokens': <class ",
            "'int'>}",
        )),
        (ac.AgentCoreMetrics.__required_keys__, ac.AgentCoreMetrics.__optional_keys__,
         ac.AgentCoreMetrics, (
            "required: latencyMs | optional:  | annotations: {'latencyMs': <class 'int'>}",
        )),
        (ac.AgentCoreMetadata.__required_keys__, ac.AgentCoreMetadata.__optional_keys__,
         ac.AgentCoreMetadata, (
            "required: metrics usage | optional:  | annotations: {'usage': <class ",
            "'AgentCoreUsage'>, 'metrics': <class 'AgentCoreMetrics'>}",
        )),
        (ac.AgentCoreEventPayload.__required_keys__, ac.AgentCoreEventPayload.__optional_keys__,
         ac.AgentCoreEventPayload, (
            "required:  | optional: contentBlockDelta contentBlockStop messageStart messageStop ",
            "metadata | annotations: {'messageStart': AgentCoreMessageStart | None, ",
            "'contentBlockDelta': AgentCoreContentBlockDeltaEvent | None, 'contentBlockStop': ",
            "AgentCoreContentBlockStop | None, 'messageStop': AgentCoreMessageStop | None, ",
            "'metadata': AgentCoreMetadata | None}",
        )),
        (ac.AgentCoreEvent.__required_keys__, ac.AgentCoreEvent.__optional_keys__,
         ac.AgentCoreEvent, (
            "required:  | optional: event | annotations: {'event': AgentCoreEventPayload | None}",
        )),
        (ac.AgentCoreContentBlock.__required_keys__, ac.AgentCoreContentBlock.__optional_keys__,
         ac.AgentCoreContentBlock, (
            "required: text | optional:  | annotations: {'text': <class 'str'>}",
        )),
        (ac.AgentCoreMessage.__required_keys__, ac.AgentCoreMessage.__optional_keys__,
         ac.AgentCoreMessage, (
            "required: content role | optional:  | annotations: {'role': Literal['assistant'], ",
            "'content': list[AgentCoreContentBlock]}",
        )),
        (ac.AgentCoreFinalMessage.__required_keys__, ac.AgentCoreFinalMessage.__optional_keys__,
         ac.AgentCoreFinalMessage, (
            "required: message | optional:  | annotations: {'message': <class ",
            "'AgentCoreMessage'>}",
        )),
        (ac.AgentCoreParsedResponse.__required_keys__, ac.AgentCoreParsedResponse.__optional_keys__,
         ac.AgentCoreParsedResponse, (
            "required: content final_message usage | optional:  | annotations: {'content': ",
            "<class 'str'>, 'usage': AgentCoreUsage | None, 'final_message': AgentCoreMessage | ",
            "None}",
        )),
        (ia.InvokeAgentEventHeaders.__required_keys__, ia.InvokeAgentEventHeaders.__optional_keys__,
         ia.InvokeAgentEventHeaders, (
            "required:  | optional: content_type event_type message_type | annotations: ",
            "{'event_type': <class 'str'>, 'content_type': <class 'str'>, 'message_type': <class ",
            "'str'>}",
        )),
        (ia.InvokeAgentUsage.__required_keys__, ia.InvokeAgentUsage.__optional_keys__,
         ia.InvokeAgentUsage, (
            "required: inputTokens model outputTokens | optional:  | annotations: ",
            "{'inputTokens': <class 'int'>, 'outputTokens': <class 'int'>, 'model': str | None}",
        )),
        (ia.InvokeAgentMetadata.__required_keys__, ia.InvokeAgentMetadata.__optional_keys__,
         ia.InvokeAgentMetadata, (
            "required:  | optional: clientRequestId endTime startTime totalTimeMs usage | ",
            "annotations: {'clientRequestId': str | None, 'endTime': str | None, 'startTime': ",
            "str | None, 'totalTimeMs': int | None, 'usage': InvokeAgentUsage | None}",
        )),
        (ia.InvokeAgentModelInvocationInput.__required_keys__,
         ia.InvokeAgentModelInvocationInput.__optional_keys__,
         ia.InvokeAgentModelInvocationInput, (
            "required:  | optional: foundationModel inferenceConfiguration text traceId type | ",
            "annotations: {'foundationModel': str | None, 'inferenceConfiguration': dict[str, ",
            "JsonValue] | None, 'text': str | None, 'traceId': str | None, 'type': str | None}",
        )),
        (ia.InvokeAgentModelInvocationOutput.__required_keys__,
         ia.InvokeAgentModelInvocationOutput.__optional_keys__,
         ia.InvokeAgentModelInvocationOutput, (
            "required:  | optional: metadata parsedResponse rawResponse reasoningContent traceId ",
            "| annotations: {'metadata': InvokeAgentMetadata | None, 'parsedResponse': dict[str, ",
            "JsonValue] | None, 'rawResponse': dict[str, JsonValue] | None, 'reasoningContent': ",
            "dict[str, JsonValue] | None, 'traceId': str | None}",
        )),
        (ia.InvokeAgentOrchestrationTrace.__required_keys__,
         ia.InvokeAgentOrchestrationTrace.__optional_keys__,
         ia.InvokeAgentOrchestrationTrace, (
            "required:  | optional: modelInvocationInput modelInvocationOutput | annotations: ",
            "{'modelInvocationInput': InvokeAgentModelInvocationInput | None, ",
            "'modelInvocationOutput': InvokeAgentModelInvocationOutput | None}",
        )),
        (ia.InvokeAgentPreProcessingTrace.__required_keys__,
         ia.InvokeAgentPreProcessingTrace.__optional_keys__,
         ia.InvokeAgentPreProcessingTrace, (
            "required:  | optional: modelInvocationInput modelInvocationOutput | annotations: ",
            "{'modelInvocationInput': InvokeAgentModelInvocationInput | None, ",
            "'modelInvocationOutput': InvokeAgentModelInvocationOutput | None}",
        )),
        (ia.InvokeAgentTrace.__required_keys__, ia.InvokeAgentTrace.__optional_keys__,
         ia.InvokeAgentTrace, (
            "required:  | optional: orchestrationTrace preProcessingTrace | annotations: ",
            "{'orchestrationTrace': InvokeAgentOrchestrationTrace | None, 'preProcessingTrace': ",
            "InvokeAgentPreProcessingTrace | None}",
        )),
        (ia.InvokeAgentCallerChain.__required_keys__, ia.InvokeAgentCallerChain.__optional_keys__,
         ia.InvokeAgentCallerChain, (
            "required:  | optional: agentAliasArn | annotations: {'agentAliasArn': <class 'str'>}",
        )),
        (ia.InvokeAgentTracePayload.__required_keys__, ia.InvokeAgentTracePayload.__optional_keys__,
         ia.InvokeAgentTracePayload, (
            "required:  | optional: agentAliasId agentId agentVersion callerChain eventTime ",
            "sessionId trace | annotations: {'agentAliasId': <class 'str'>, 'agentId': <class ",
            "'str'>, 'agentVersion': <class 'str'>, 'callerChain': list[InvokeAgentCallerChain], ",
            "'eventTime': <class 'str'>, 'sessionId': <class 'str'>, 'trace': <class ",
            "'InvokeAgentTrace'>}",
        )),
        (ia.InvokeAgentChunkPayload.__required_keys__, ia.InvokeAgentChunkPayload.__optional_keys__,
         ia.InvokeAgentChunkPayload, (
            "required:  | optional: bytes | annotations: {'bytes': <class 'str'>}",
        )),
        (ia.InvokeAgentEventPayload.__required_keys__, ia.InvokeAgentEventPayload.__optional_keys__,
         ia.InvokeAgentEventPayload, (
            "required:  | optional: agentAliasId agentId agentVersion bytes callerChain ",
            "eventTime sessionId trace | annotations: {'agentAliasId': str | None, 'agentId': ",
            "str | None, 'agentVersion': str | None, 'callerChain': list[InvokeAgentCallerChain] ",
            "| None, 'eventTime': str | None, 'sessionId': str | None, 'trace': InvokeAgentTrace ",
            "| None, 'bytes': str | None}",
        )),
        (ia.InvokeAgentEvent.__required_keys__, ia.InvokeAgentEvent.__optional_keys__,
         ia.InvokeAgentEvent, (
            "required:  | optional: headers payload | annotations: {'headers': <class ",
            "'InvokeAgentEventHeaders'>, 'payload': InvokeAgentEventPayload | None}",
        )),
    )
    for required, optional, cls, chunks in cases:
        annotations = MODULE_PATH.sub("", str(cls.__annotations__))
        expected = "required: " + " ".join(sorted(required))
        expected += " | optional: " + " ".join(sorted(optional))
        expected += " | annotations: " + annotations
        assert expected == "".join(chunks), cls.__name__


def test_aws_auth_params_names() -> None:
    params = batches.AwsAuthParams
    config = params.model_config
    assert config.get("extra") == "ignore"
    assert config.get("frozen") is True
    fields = "fields: " + MODULE_PATH.sub("", str(params.model_fields))
    expected_fields = (
        "fields: {'aws_access_key_id': FieldInfo(annotation=Union[str, NoneType], ",
        "required=False, default=None), 'aws_secret_access_key': ",
        "FieldInfo(annotation=Union[str, NoneType], required=False, default=None), ",
        "'aws_session_token': FieldInfo(annotation=Union[str, NoneType], required=False, ",
        "default=None), 'aws_session_name': FieldInfo(annotation=Union[str, NoneType], ",
        "required=False, default=None), 'aws_profile_name': FieldInfo(annotation=Union[str, ",
        "NoneType], required=False, default=None), 'aws_role_name': ",
        "FieldInfo(annotation=Union[str, NoneType], required=False, default=None), ",
        "'aws_web_identity_token': FieldInfo(annotation=Union[str, NoneType], ",
        "required=False, default=None), 'aws_sts_endpoint': FieldInfo(annotation=Union[str, ",
        "NoneType], required=False, default=None), 'aws_external_id': ",
        "FieldInfo(annotation=Union[str, NoneType], required=False, default=None), ",
        "'aws_session_tags': FieldInfo(annotation=Union[Sequence[AwsSessionTag], JsonValue], ",
        "required=False, default=None)}",
    )
    assert fields == "".join(expected_fields)
    request: dict[str, str] = {"aws_access_key_id": "a", "unexpected": "dropped"}
    ported = params.model_validate(request)
    assert ported.aws_access_key_id == "a"
    assert ported.aws_session_tags is None
    expected_ported = {
        "aws_access_key_id": "a",
        "aws_secret_access_key": None,
        "aws_session_token": None,
        "aws_session_name": None,
        "aws_profile_name": None,
        "aws_role_name": None,
        "aws_web_identity_token": None,
        "aws_sts_endpoint": None,
        "aws_external_id": None,
        "aws_session_tags": None,
    }
    dumped = ported.model_dump_json()
    assert dumped == json.dumps(expected_ported, separators=(",", ":"))
    tags: Sequence[batches.AwsSessionTag] = ({"Key": "team", "Value": "genai"},)
    tagged_request: dict[str, Sequence[batches.AwsSessionTag]] = {"aws_session_tags": tags}
    tagged = params.model_validate(tagged_request)
    assert tagged.aws_session_tags == tags
    expected_tagged = {
        "aws_access_key_id": None,
        "aws_secret_access_key": None,
        "aws_session_token": None,
        "aws_session_name": None,
        "aws_profile_name": None,
        "aws_role_name": None,
        "aws_web_identity_token": None,
        "aws_sts_endpoint": None,
        "aws_external_id": None,
        "aws_session_tags": [{"Key": "team", "Value": "genai"}],
    }
    tagged_dump = tagged.model_dump_json()
    assert tagged_dump == json.dumps(expected_tagged, separators=(",", ":"))


def test_aliases() -> None:
    emb = embeddings
    mar = marengo
    nov = nova_embeddings
    bat = batches
    ia = bedrock_invoke_agents
    cases = (
        (str(emb.COHERE_EMBEDDING_INPUT_TYPES), (
            "Literal['search_document', 'search_query', 'classification', 'clustering', 'image']",
        )),
        (str(mar.TWELVELABS_EMBEDDING_INPUT_TYPES), (
            "Literal['text', 'image', 'video', 'audio']",
        )),
        (str(mar.TWELVELABS_EMBEDDING_OPTIONS), (
            "Literal['visual-text', 'visual-image', 'audio']",
        )),
        (str(mar.TWELVELABS_MARENGO_3_INPUT_TYPES), (
            "Literal['text', 'image', 'video', 'audio', 'text_image', 'multi_input']",
        )),
        (str(mar.TWELVELABS_MARENGO_3_EMBEDDING_OPTIONS), (
            "Literal['visual', 'audio', 'transcription']",
        )),
        (str(mar.TWELVELABS_MARENGO_3_EMBEDDING_TYPES), (
            "Literal['separate_embedding', 'fused_embedding']",
        )),
        (str(mar.TWELVELABS_MARENGO_3_EMBEDDING_SCOPES), (
            "Literal['clip', 'asset']",
        )),
        (str(mar.TwelveLabsMarengo3Segmentation), (
            "TwelveLabsMarengo3FixedSegmentation | TwelveLabsMarengo3DynamicSegmentation",
        )),
        (str(mar.TwelveLabsMarengo3EmbeddingRequest), (
            "TwelveLabsMarengo3TextRequest | TwelveLabsMarengo3ImageRequest | ",
            "TwelveLabsMarengo3VideoRequest | TwelveLabsMarengo3AudioRequest | ",
            "TwelveLabsMarengo3TextImageRequest | TwelveLabsMarengo3MultiInputRequest",
        )),
        (str(nov.NOVA_EMBEDDING_PURPOSES), (
            "Literal['GENERIC_INDEX', 'GENERIC_RETRIEVAL', 'TEXT_RETRIEVAL', 'IMAGE_RETRIEVAL', ",
            "'VIDEO_RETRIEVAL', 'DOCUMENT_RETRIEVAL', 'AUDIO_RETRIEVAL', 'CLASSIFICATION', ",
            "'CLUSTERING']",
        )),
        (str(nov.NOVA_EMBEDDING_DIMENSIONS), (
            "Literal[256, 384, 1024, 3072]",
        )),
        (str(nov.NOVA_TRUNCATION_MODES), (
            "Literal['START', 'END', 'NONE']",
        )),
        (str(nov.NOVA_DETAIL_LEVELS), (
            "Literal['STANDARD_IMAGE', 'DOCUMENT_IMAGE']",
        )),
        (str(nov.NOVA_EMBEDDING_MODES), (
            "Literal['AUDIO_VIDEO_COMBINED', 'AUDIO_VIDEO_SEPARATE']",
        )),
        (str(nov.NOVA_EMBEDDING_TYPES), (
            "Literal['TEXT', 'IMAGE', 'VIDEO', 'AUDIO', 'AUDIO_VIDEO_COMBINED']",
        )),
        (str(nov.AmazonEmbeddingRequest), (
            "AmazonTitanMultimodalEmbeddingRequest | AmazonTitanV2EmbeddingRequest | ",
            "AmazonTitanG1EmbeddingRequest",
        )),
        (str(bat.BedrockBatchJobStatus), (
            "Literal['Submitted', 'InProgress', 'Completed', 'Failed', 'Stopping', 'Stopped']",
        )),
        (str(ia.InvokeAgentEventList), (
            "list[InvokeAgentEvent]",
        )),
        (str(ia.InvokeAgentTraceEvent), (
            "<class 'InvokeAgentEvent'>",
        )),
        (str(ia.InvokeAgentChunkEvent), (
            "<class 'InvokeAgentEvent'>",
        )),
    )
    for actual, chunks in cases:
        assert MODULE_PATH.sub("", actual) == "".join(chunks)
    assert batches.AWS_AUTH_PARAM_KEYS == (
        "aws_access_key_id",
        "aws_secret_access_key",
        "aws_session_token",
        "aws_session_name",
        "aws_profile_name",
        "aws_role_name",
        "aws_web_identity_token",
        "aws_sts_endpoint",
        "aws_external_id",
        "aws_session_tags",
    )
    assert [repr(member) for member in batches.BedrockBatchRecordKind] == [
        "<BedrockBatchRecordKind.CHAT: 'chat'>",
        "<BedrockBatchRecordKind.TEXT_COMPLETION: 'text_completion'>",
        "<BedrockBatchRecordKind.RESPONSES: 'responses'>",
        "<BedrockBatchRecordKind.EMBEDDING: 'embedding'>",
    ]
