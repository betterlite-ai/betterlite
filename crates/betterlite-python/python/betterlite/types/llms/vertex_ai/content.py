from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class FileDataType(TypedDict):
    mime_type: str
    file_uri: str


class BlobType(TypedDict, total=False):
    mime_type: Required[str]
    data: Required[str]


class FunctionResponsePartType(TypedDict, total=False):
    inline_data: BlobType
    file_data: FileDataType


class FunctionResponse(TypedDict, total=False):
    id: str
    name: Required[str]
    response: dict[str, JsonValue] | None
    parts: list[FunctionResponsePartType]


class FunctionCall(TypedDict, total=False):
    id: str
    name: Required[str]
    args: dict[str, JsonValue] | None


class PartType(TypedDict, total=False):
    text: str
    inline_data: BlobType
    file_data: FileDataType
    function_call: FunctionCall
    function_response: FunctionResponse
    thought: bool
    thoughtSignature: str
    media_resolution: Literal["low", "medium", "high"]


class HttpxFunctionCall(TypedDict, total=False):
    id: str
    name: Required[str]
    args: dict[str, JsonValue]


class HttpxExecutableCode(TypedDict):
    code: str
    language: str


class HttpxCodeExecutionResult(TypedDict):
    outcome: str
    output: str


class HttpxBlobType(TypedDict, total=False):
    mimeType: str
    data: str


class HttpxServerSideToolCall(TypedDict, total=False):
    toolType: str
    id: str
    args: dict[str, JsonValue]


class HttpxServerSideToolResponse(TypedDict, total=False):
    toolType: str
    id: str
    response: str | dict[str, JsonValue]


class HttpxPartType(TypedDict, total=False):
    text: str
    inlineData: HttpxBlobType
    fileData: FileDataType
    functionCall: HttpxFunctionCall
    functionResponse: FunctionResponse
    toolCall: HttpxServerSideToolCall
    toolResponse: HttpxServerSideToolResponse
    executableCode: HttpxExecutableCode
    codeExecutionResult: HttpxCodeExecutionResult
    thought: bool
    thoughtSignature: str
    mediaResolution: Literal["low", "medium", "high"]


class HttpxContentType(TypedDict, total=False):
    role: Literal["user", "model"]
    parts: list[HttpxPartType]


class ContentType(TypedDict, total=False):
    role: Literal["user", "model"]
    parts: Required[list[PartType]]


class SystemInstructions(TypedDict):
    parts: Required[list[PartType]]
