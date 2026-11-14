from typing import Literal

from typing_extensions import TypedDict


class GcsSource(TypedDict):
    uris: list[str]


class InputConfig(TypedDict):
    instancesFormat: str
    gcsSource: GcsSource


class GcsDestination(TypedDict):
    outputUriPrefix: str


class OutputConfig(TypedDict, total=False):
    predictionsFormat: str
    gcsDestination: GcsDestination


class OutputInfo(TypedDict, total=False):
    gcsOutputDirectory: str


class GcsBucketResponse(TypedDict):
    kind: Literal["storage#object"]
    id: str
    selfLink: str
    mediaLink: str
    name: str
    bucket: str
    generation: str
    metageneration: str
    contentType: str
    storageClass: str
    size: str
    md5Hash: str
    crc32c: str
    etag: str
    timeCreated: str
    updated: str
    timeStorageClassUpdated: str
    timeFinalized: str


class VertexAIBatchPredictionJob(TypedDict):
    displayName: str
    model: str
    inputConfig: InputConfig
    outputConfig: OutputConfig


class VertexBatchPredictionResponse(TypedDict, total=False):
    name: str
    displayName: str
    model: str
    inputConfig: InputConfig
    outputConfig: OutputConfig
    outputInfo: OutputInfo
    state: str
    createTime: str
    updateTime: str
    modelVersionId: str
