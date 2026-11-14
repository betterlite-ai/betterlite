import asyncio
from collections.abc import Awaitable, Callable, Mapping
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict

from litellm.exceptions import RateLimitError
from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.types.integrations.custom_logger import AgenticLoopPlan
from litellm.types.integrations.websearch_interception import RichWebSearchInput
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# the case never streams a follow-up call, and LiteLLM's `stream` is positional
NO_STREAM: Final = False
# the logging object only carries the call id and the agentic-loop model, so the start time is fixed
START_TIME: Final = datetime(2024, 1, 1, tzinfo=UTC)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

PlanCall = Callable[
    [
        dict[str, JsonValue],
        str,
        list[dict[str, JsonValue]],
        None,
        None,
        dict[str, JsonValue],
        Logging | None,
        bool,
        dict[str, JsonValue],
    ],
    Awaitable[AgenticLoopPlan],
]


class SearchScript(TypedDict):
    kind: Literal["success", "rate_limit"]
    text: NotRequired[str]
    message: NotRequired[str]


class LoggingSpec(TypedDict):
    model_call_details: dict[str, JsonValue]


class Case(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    tool_calls: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    searches: list[SearchScript]
    thinking_blocks: NotRequired[list[dict[str, JsonValue]]]
    logging: NotRequired[LoggingSpec]


class Input(TypedDict):
    cases: list[Case]


class PlanBuilder(Protocol):
    searches: list[str]

    @property
    def async_build_agentic_loop_plan(self) -> PlanCall: ...


class AgenticLogging(Protocol):
    model_call_details: dict[str, JsonValue]


class ScriptedLogger(WebSearchInterceptionLogger):
    def __init__(self, scripts: list[SearchScript]) -> None:
        super().__init__()
        self.scripts = scripts
        self.searches: list[str] = []

    @override
    async def _execute_search(
        self,
        query: str,
        # ast-grep-ignore: no-loose-types
        kwargs: Mapping[str, object] | None = None,
        rich: RichWebSearchInput | None = None,
    ) -> tuple[str, SearchResponse | None]:
        del kwargs, rich
        self.searches.append(query)
        if not self.scripts:
            raise ValueError("the case scripts no search response")
        script = self.scripts.pop(0)
        if script["kind"] == "rate_limit":
            message = script.get("message")
            raise RateLimitError(
                message if message is not None else "",
                llm_provider="anthropic",
                model="claude-sonnet-4-5",
            )
        text = script.get("text")
        return text if text is not None else "", None


def thinking_blocks(case: Case) -> list[dict[str, JsonValue]]:
    blocks = case.get("thinking_blocks")
    if blocks is None:
        return []
    return blocks


def logging_for(case: Case) -> Logging | None:
    spec = case.get("logging")
    if spec is None:
        return None
    logging_obj = Logging(
        model=case["model"],
        messages=None,
        stream=NO_STREAM,
        call_type="acompletion",
        start_time=START_TIME,
        litellm_call_id="call-1",
        function_id="fn",
    )
    seeded: AgenticLogging = logging_obj
    seeded.model_call_details["agentic_loop_params"] = spec["model_call_details"]
    return logging_obj


async def build_plan(builder: PlanBuilder, case: Case) -> AgenticLoopPlan:
    tools: dict[str, JsonValue] = {
        "tool_calls": JSON.validate_python(case["tool_calls"]),
        "thinking_blocks": JSON.validate_python(thinking_blocks(case)),
    }
    return await builder.async_build_agentic_loop_plan(
        tools,
        case["model"],
        case["messages"],
        None,
        None,
        case["optional_params"],
        logging_for(case),
        NO_STREAM,
        case["kwargs"],
    )


async def plan_result(case: Case) -> JsonValue:
    logger = ScriptedLogger(case["searches"])
    plan = await build_plan(logger, case)
    return {
        "plan": JSON.validate_json(plan.model_dump_json()),
        "searches": JSON.validate_python(logger.searches),
    }


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = [
        asyncio.run(plan_result(case)) for case in input_value["cases"]
    ]
    return {"results": results}
