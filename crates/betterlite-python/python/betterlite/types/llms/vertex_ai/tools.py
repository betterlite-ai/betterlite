from enum import StrEnum
from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.vertex_ai.schema import FunctionDeclaration


class VertexAISearch(TypedDict, total=False):
    datastore: Required[str]


class Retrieval(TypedDict):
    source: VertexAISearch


class FunctionCallingConfig(TypedDict, total=False):
    mode: Literal["ANY", "AUTO", "NONE"]
    allowed_function_names: list[str]


class VertexToolName(StrEnum):
    GOOGLE_SEARCH = "googleSearch"
    GOOGLE_SEARCH_RETRIEVAL = "googleSearchRetrieval"
    ENTERPRISE_WEB_SEARCH = "enterpriseWebSearch"
    URL_CONTEXT = "url_context"
    CODE_EXECUTION = "code_execution"
    GOOGLE_MAPS = "googleMaps"
    COMPUTER_USE = "computerUse"


class Tools(TypedDict, total=False):
    function_declarations: list[FunctionDeclaration]
    googleSearch: dict[str, JsonValue]
    googleSearchRetrieval: dict[str, JsonValue]
    enterpriseWebSearch: dict[str, JsonValue]
    url_context: dict[str, JsonValue]
    code_execution: dict[str, JsonValue]
    googleMaps: dict[str, JsonValue]
    computerUse: dict[str, JsonValue]
    retrieval: Retrieval


class ToolConfig(TypedDict, total=False):
    functionCallingConfig: FunctionCallingConfig
    includeServerSideToolInvocations: bool
