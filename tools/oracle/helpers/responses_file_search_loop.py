import asyncio
import logging
from typing import Final, Literal, TypeAlias, TypedDict

import litellm.vector_stores.main as vector_stores_main
from litellm.responses import main as responses_main
from litellm.responses.file_search import emulated_handler
from litellm.types.llms.openai import ResponsesAPIResponse
from openai.types.responses.tool_param import ToolParam
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LOG_NAME: Final = "LiteLLM"
JSON: Final = TypeAdapter(JsonValue)
RESPONSE: Final = TypeAdapter(ResponsesAPIResponse)
UNSCRIPTED: Final = "the case scripts no such search"
RESERVED: Final = frozenset({"input", "model", "tools"})

Object: TypeAlias = dict[str, JsonValue]


class ResultSearch(TypedDict):
    kind: Literal["result"]
    vector_store_id: str
    query: str
    response: Object


class ErrorSearch(TypedDict):
    kind: Literal["error"]
    vector_store_id: str
    query: str
    error: str


Search: TypeAlias = ResultSearch | ErrorSearch


class LoopCall(TypedDict):
    input: JsonValue
    model: str
    tools: list[ToolParam] | None
    kwargs: dict[str, JsonValue]
    provider: list[Object]
    searches: list[Search]


class ScriptedError(Exception):
    pass


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append({"level": record.levelname, "message": record.getMessage()})


class ScriptedResponses:
    def __init__(self, provider: list[Object]) -> None:
        self.responses: list[ResponsesAPIResponse] = [
            RESPONSE.validate_python(payload) for payload in provider
        ]
        self.calls: list[JsonValue] = []

    async def __call__(self, **kwargs: JsonValue) -> ResponsesAPIResponse:
        self.calls.append(
            {
                "input": kwargs["input"],
                "model": kwargs["model"],
                "tools": kwargs["tools"],
                "kwargs": {key: value for key, value in kwargs.items() if key not in RESERVED},
            }
        )
        return self.responses.pop(0)


class ScriptedSearches:
    def __init__(self, searches: list[Search]) -> None:
        self.searches = searches
        self.requests: list[JsonValue] = []

    async def __call__(self, *, vector_store_id: str, query: str) -> JsonValue:
        self.requests.append({"vector_store_id": vector_store_id, "query": query})
        for search in self.searches:
            if search["vector_store_id"] == vector_store_id and search["query"] == query:
                if search["kind"] == "result":
                    return search["response"]
                raise ScriptedError(search["error"])
        raise ScriptedError(UNSCRIPTED)


def run(input_value: LoopCall) -> JsonValue:
    fake_responses = ScriptedResponses(input_value["provider"])
    fake_searches = ScriptedSearches(input_value["searches"])
    object.__setattr__(responses_main, "aresponses", fake_responses)
    object.__setattr__(vector_stores_main, "asearch", fake_searches)
    handler = Capture()
    logger = logging.getLogger(LOG_NAME)
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(handler)
    logger.setLevel(logging.WARNING)
    try:
        response = asyncio.run(
            emulated_handler.aresponses_with_emulated_file_search(
                input=input_value["input"],
                model=input_value["model"],
                tools=input_value["tools"],
                **input_value["kwargs"],
            )
        )
    finally:
        logger.removeHandler(handler)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    return {
        "response": JSON.validate_json(response.model_dump_json()),
        "calls": fake_responses.calls,
        "searches": fake_searches.requests,
        "diagnostics": handler.lines,
    }
