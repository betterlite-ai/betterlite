import asyncio
from collections.abc import Mapping
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.types.integrations.custom_logger import AgenticLoopPlan
from litellm.types.integrations.websearch_interception import RichWebSearchInput
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class SearchScript(TypedDict):
    kind: Literal["success", "error"]
    text: NotRequired[str]
    message: NotRequired[str]


class ChatCase(TypedDict):
    operation: Literal["build_chat_plan", "build_chat_rerun"]
    model: str
    tool_calls: list[dict[str, JsonValue]]
    searches: list[SearchScript]
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    response_format: NotRequired[str]


class ResponsesCase(TypedDict):
    operation: Literal["build_responses_plan"]
    model: str
    tool_calls: list[dict[str, JsonValue]]
    searches: list[SearchScript]
    input: str | list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]


class PatchDump(TypedDict):
    model: str | None
    messages: list[JsonValue] | None
    tools: list[JsonValue] | None
    max_tokens: int | None
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]


class PlanDump(TypedDict):
    run_agentic_loop: bool
    request_patch: PatchDump | None
    response_override: JsonValue
    terminate: bool
    stop_reason: str | None
    metadata: dict[str, JsonValue]


class RecordedSearch(TypedDict):
    query: str
    rich: JsonValue


Input = ChatCase | ResponsesCase

PLAN: TypeAdapter[PlanDump] = TypeAdapter(PlanDump)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
RECORDED: list[dict[str, JsonValue]] = []


class ScriptedLogger(WebSearchInterceptionLogger):
    def __init__(self, scripts: list[SearchScript]) -> None:
        super().__init__()
        self.scripts = scripts
        self.searches: list[RecordedSearch] = []

    @override
    async def _execute_search(
        self,
        query: str,
        kwargs: Mapping[str, object] | None = None,
        rich: RichWebSearchInput | None = None,
    ) -> tuple[str, SearchResponse | None]:
        del kwargs
        self.searches.append({"query": query, "rich": rich_value(rich)})
        if not self.scripts:
            raise ValueError("the case scripts no search response")
        script = self.scripts.pop(0)
        if script["kind"] == "error":
            message = script.get("message")
            raise ValueError(message if message is not None else "")
        text = script.get("text")
        return (text if text is not None else ""), None


def rich_value(rich: RichWebSearchInput | None) -> JsonValue:
    if rich is None:
        return None
    value: dict[str, JsonValue] = {}
    objective = rich.get("objective")
    if objective is not None:
        value["objective"] = objective
    queries = rich.get("search_queries")
    if queries is not None:
        value["search_queries"] = list(queries)
    return value


def patch_value(patch: PatchDump) -> dict[str, JsonValue]:
    return {
        "model": patch["model"],
        "messages": patch["messages"],
        "tools": patch["tools"],
        "max_tokens": patch["max_tokens"],
        "optional_params": patch["optional_params"],
        "kwargs": patch["kwargs"],
    }


def plan_value(plan: AgenticLoopPlan) -> JsonValue:
    dumped = PLAN.validate_json(plan.model_dump_json())
    patch = dumped["request_patch"]
    return {
        "run_agentic_loop": dumped["run_agentic_loop"],
        "request_patch": None if patch is None else patch_value(patch),
        "response_override": dumped["response_override"],
        "terminate": dumped["terminate"],
        "stop_reason": dumped["stop_reason"],
        "metadata": dumped["metadata"],
    }


async def build_chat_plan(
    logger: ScriptedLogger,
    input_value: ChatCase,
) -> JsonValue:
    tools: dict[str, JsonValue] = {
        "tool_calls": JSON.validate_python(input_value["tool_calls"]),
    }
    response_format = input_value.get("response_format")
    if response_format is not None:
        tools["response_format"] = response_format
    plan = await logger.async_build_chat_completion_agentic_loop_plan(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1127-L1145
        tools=tools,
        model=input_value["model"],
        messages=input_value["messages"],
        response=None,
        optional_params=input_value["optional_params"],
        logging_obj=None,
        stream=False,
        kwargs=input_value["kwargs"],
    )
    return {
        "result": plan_value(plan),
        "searches": JSON.validate_python(logger.searches),
    }


async def build_responses_plan(
    logger: ScriptedLogger,
    input_value: ResponsesCase,
) -> JsonValue:
    tools: dict[str, JsonValue] = {
        "tool_calls": JSON.validate_python(input_value["tool_calls"]),
    }
    plan = await logger.async_build_responses_agentic_loop_plan(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1154-L1177
        tools=tools,
        model=input_value["model"],
        messages=input_value["input"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1154-L1162
        response=None,
        optional_params=input_value["optional_params"],
        logging_obj=None,
        stream=False,
        kwargs=input_value["kwargs"],
    )
    return {
        "result": plan_value(plan),
        "searches": JSON.validate_python(logger.searches),
    }


async def build_chat_rerun(
    logger: ScriptedLogger,
    input_value: ChatCase,
) -> JsonValue:
    tools: dict[str, JsonValue] = {
        "tool_calls": JSON.validate_python(input_value["tool_calls"]),
    }
    await logger.async_run_chat_completion_agentic_loop(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1092-L1125
        tools=tools,
        model=input_value["model"],
        messages=input_value["messages"],
        response=None,
        optional_params=input_value["optional_params"],
        logging_obj=None,
        stream=False,
        kwargs=input_value["kwargs"],
    )
    return {
        "result": RECORDED[-1] if RECORDED else None,
        "searches": JSON.validate_python(logger.searches),
    }


async def record_acompletion(
    model: str,
    messages: list[JsonValue],
    **kwargs: JsonValue,
) -> ModelResponse:
    RECORDED.append({"model": model, "messages": list(messages), **kwargs})
    return ModelResponse()


def run(input_value: Input) -> JsonValue:
    litellm.acompletion = record_acompletion
    RECORDED.clear()
    logger = ScriptedLogger(input_value["searches"])
    if input_value["operation"] == "build_responses_plan":
        return asyncio.run(build_responses_plan(logger, input_value))
    if input_value["operation"] == "build_chat_plan":
        return asyncio.run(build_chat_plan(logger, input_value))
    return asyncio.run(build_chat_rerun(logger, input_value))
