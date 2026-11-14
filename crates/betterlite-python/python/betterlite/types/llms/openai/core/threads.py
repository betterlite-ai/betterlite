from collections.abc import Iterable
from typing import Literal, Required, TypeAlias, TypedDict

from pydantic import JsonValue

from betterlite.types.utils.base import ResponseObject


class ToolResourcesCodeInterpreter(TypedDict, total=False):
    file_ids: list[str]


# ast-grep-ignore: no-long-names
class ToolResourcesFileSearchVectorStore(TypedDict, total=False):
    file_ids: list[str]
    metadata: JsonValue


class ToolResourcesFileSearch(TypedDict, total=False):
    vector_store_ids: list[str]
    # ast-grep-ignore: no-long-names
    vector_stores: Iterable[ToolResourcesFileSearchVectorStore]


# ast-grep-ignore: no-long-names
class OpenAICreateThreadParamsToolResources(TypedDict, total=False):
    code_interpreter: ToolResourcesCodeInterpreter
    file_search: ToolResourcesFileSearch


class FileSearchToolParam(TypedDict, total=False):
    type: Required[Literal["file_search"]]


class CodeInterpreterToolParam(TypedDict, total=False):
    type: Required[Literal["code_interpreter"]]


AttachmentTool: TypeAlias = CodeInterpreterToolParam | FileSearchToolParam


class Attachment(TypedDict, total=False):
    file_id: str
    tools: Iterable[AttachmentTool]


class ImageFileObject(TypedDict):
    file_id: Required[str]
    detail: str | None


class ImageURLObject(TypedDict, total=False):
    url: Required[str]
    detail: str | None


class ImageURLListItem(TypedDict):
    image_url: ImageURLObject
    index: int
    type: Literal["image_url"]


class MessageContentTextObject(TypedDict):
    type: Required[Literal["text"]]
    text: str


class MessageContentImageFileObject(TypedDict):
    type: Literal["image_file"]
    image_file: ImageFileObject


class MessageContentImageURLObject(TypedDict):
    type: Required[str]
    image_url: ImageURLObject


class MessageData(TypedDict):
    role: Literal["user", "assistant"]
    content: str | list[
        MessageContentTextObject | MessageContentImageFileObject | MessageContentImageURLObject
    ]
    attachments: list[Attachment] | None
    metadata: dict[str, JsonValue] | None


class Thread(ResponseObject):
    id: str
    created_at: int
    metadata: JsonValue = None
    object: Literal["thread"]
