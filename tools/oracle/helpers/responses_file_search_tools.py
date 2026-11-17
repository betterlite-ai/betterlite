import logging
import time
import uuid
from typing import Final, Literal, TypeAlias, TypedDict, cast
from uuid import UUID

from litellm.responses.file_search import emulated_handler
from litellm.responses.file_search.emulated_handler import FileSearchToolCallArgs
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.vector_stores import VectorStoreSearchResult
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LOG_NAME: Final = "LiteLLM"
JSON: Final = TypeAdapter(JsonValue)
ORIGINAL_ERROR: Final[dict[str, str]] = {"code": "server_error", "message": "the provider call failed"}

Object: TypeAlias = dict[str, JsonValue]


class ToolSchemaCall(TypedDict):
    function: Literal["tool_schema"]
    vector_store_ids: list[str]


class ToolReplacementCall(TypedDict):
    function: Literal["tool_replacement"]
    tools: list[JsonValue] | None


class RenderingCall(TypedDict):
    function: Literal["result_rendering"]
    results: list[VectorStoreSearchResult]


class OutputItemsCall(TypedDict):
    function: Literal["output_items"]
    call_id: str
    queries: list[str]
    include_search_results: bool
    results: list[VectorStoreSearchResult] | None
    text: str


class OriginalView(TypedDict):
    id: str
    created_at: int
    model: str | None
    usage: Object | None
    _hidden_params: Object | None


class SynthesisCall(TypedDict):
    function: Literal["response_synthesis"]
    call_id: str
    queries: list[str]
    include_search_results: bool
    results: list[VectorStoreSearchResult]
    text: str
    original: OriginalView | None
    _hidden_params: Object | None
    uuid: str
    now: str


class PreparationCall(TypedDict):
    function: Literal["argument_preparation"]
    prepares: list[Object]
    fallback_call_id: str
    tool_calls: list[Object]
    query_args: list[FileSearchToolCallArgs]
    query_inputs: list[JsonValue]


Input: TypeAlias = (
    ToolSchemaCall
    | ToolReplacementCall
    | RenderingCall
    | OutputItemsCall
    | SynthesisCall
    | PreparationCall
)


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append({"level": record.levelname, "message": record.getMessage()})


class FixedClock:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> float:
        return float(self.value)


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> UUID:
        return UUID(hex=self.value)


def schema_answer(call: ToolSchemaCall) -> JsonValue:
    tool = emulated_handler._build_function_tool(call["vector_store_ids"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L65-L105
    return JSON.validate_python(tool)


def replacement_answer(call: ToolReplacementCall) -> JsonValue:
    tools, ids = emulated_handler._replace_file_search_tools(call["tools"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L108-L132
    return {"tools": JSON.validate_python(tools), "vector_store_ids": JSON.validate_python(ids)}


def rendering_answer(call: RenderingCall) -> JsonValue:
    results = call["results"]
    output = emulated_handler._format_search_results_as_tool_output(results)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L200-L225
    include = emulated_handler._build_search_results_for_include(results)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L228-L252
    return {"output": output, "include": JSON.validate_python(include)}


def items_answer(call: OutputItemsCall) -> JsonValue:
    results = call["results"]
    file_search = emulated_handler._build_file_search_call_output(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L255-L279
        call["call_id"],
        call["queries"],
        results,
        include_search_results=call["include_search_results"],
    )
    message = emulated_handler._build_message_output(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L312-L328
        call["text"],
        results if results is not None else [],
    )
    return {
        "call": JSON.validate_python(file_search),
        "message": JSON.validate_python(message),
    }


def original_response(view: OriginalView | None) -> ResponsesAPIResponse:
    if view is None:
        return cast(ResponsesAPIResponse, None)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L345-L384
    original = ResponsesAPIResponse.model_validate(
        {
            "id": view["id"],
            "created_at": view["created_at"],
            "model": view["model"],
            "output": [],
            "usage": view["usage"],
            "error": ORIGINAL_ERROR,
        }
    )
    hidden = view["_hidden_params"]
    if hidden is not None:
        object.__setattr__(original, "_hidden_params", hidden.copy())
    return original


def first_response(hidden: Object | None) -> ResponsesAPIResponse | None:
    if hidden is None:
        return None
    response = ResponsesAPIResponse.model_validate(
        {"id": "resp_first", "created_at": 0, "model": "", "output": []}
    )
    object.__setattr__(response, "_hidden_params", hidden.copy())
    return response


def synthesis_answer(call: SynthesisCall) -> JsonValue:
    first = first_response(call["_hidden_params"])
    previous_clock = time.time
    previous_uuid = uuid.uuid4
    object.__setattr__(time, "time", FixedClock(call["now"]))
    object.__setattr__(uuid, "uuid4", FixedUuid(call["uuid"]))
    try:
        file_search = emulated_handler._build_file_search_call_output(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L255-L279
            call["call_id"],
            call["queries"],
            call["results"],
            include_search_results=call["include_search_results"],
        )
        message = emulated_handler._build_message_output(call["text"], call["results"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L312-L328
        synthesized = emulated_handler._synthesize_responses_api_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L345-L384
            original_response(call["original"]),
            file_search,
            message,
            first,
        )
    finally:
        object.__setattr__(time, "time", previous_clock)
        object.__setattr__(uuid, "uuid4", previous_uuid)
    hidden = cast("Object", synthesized._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L345-L384
    return {
        "response": JSON.validate_json(synthesized.model_dump_json()),
        "hidden_params": JSON.validate_python(hidden),
    }


def prepare_one(kwargs: Object) -> Object:
    arguments = cast("dict[str, object]", kwargs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L398-L413
    handler = Capture()
    logger = logging.getLogger(LOG_NAME)
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(handler)
    logger.setLevel(logging.DEBUG)
    try:
        prepared = emulated_handler._prepare_emulated_file_search_call(arguments)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L398-L413
    finally:
        logger.removeHandler(handler)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    include_search_results, updated = prepared
    return {
        "include_search_results": include_search_results,
        "updated_kwargs": JSON.validate_python(updated),
        "logs": handler.lines,
    }


def extract_one(tool_call: Object, fallback: str) -> Object:
    call_id, arguments = emulated_handler._extract_tool_call_fields(tool_call, fallback)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L416-L425
    return {"call_id": call_id, "arguments": arguments}


def query_one(args: FileSearchToolCallArgs, input_value: JsonValue) -> JsonValue:
    queries = emulated_handler._resolve_queries_from_args(args, input_value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/file_search/emulated_handler.py#L428-L437
    return JSON.validate_python(queries)


def preparation_answer(call: PreparationCall) -> JsonValue:
    prepared = [prepare_one(case) for case in call["prepares"]]
    extracted = [
        extract_one(tool_call, call["fallback_call_id"]) for tool_call in call["tool_calls"]
    ]
    queries = [
        query_one(args, input_value)
        for args, input_value in zip(call["query_args"], call["query_inputs"], strict=True)
    ]
    return {
        "prepared": JSON.validate_python(prepared),
        "extracted": JSON.validate_python(extracted),
        "queries": JSON.validate_python(queries),
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "tool_schema":
        return schema_answer(input_value)
    if input_value["function"] == "tool_replacement":
        return replacement_answer(input_value)
    if input_value["function"] == "result_rendering":
        return rendering_answer(input_value)
    if input_value["function"] == "output_items":
        return items_answer(input_value)
    if input_value["function"] == "response_synthesis":
        return synthesis_answer(input_value)
    return preparation_answer(input_value)
