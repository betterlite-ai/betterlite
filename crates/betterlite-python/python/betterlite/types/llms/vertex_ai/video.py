from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class VertexVideoImage(TypedDict, total=False):
    bytesBase64Encoded: str
    mimeType: str


class VertexVideoGenerationInstance(TypedDict, total=False):
    prompt: Required[str]
    image: VertexVideoImage


# ast-grep-ignore: no-long-names
class VertexVideoGenerationParameters(TypedDict, total=False):
    aspectRatio: Literal["9:16", "16:9"]
    durationSeconds: int


class VertexVideoGenerationRequest(TypedDict):
    instances: Required[list[VertexVideoGenerationInstance]]
    parameters: VertexVideoGenerationParameters  # ast-grep-ignore: no-long-names


class VertexVideoOutput(TypedDict, total=False):
    bytesBase64Encoded: str
    mimeType: str
    gcsUri: str


class VertexVideoGenerationResponse(TypedDict, total=False):
    name: str
    done: bool
    response: dict[str, JsonValue]
    metadata: dict[str, JsonValue]
    error: dict[str, JsonValue]
