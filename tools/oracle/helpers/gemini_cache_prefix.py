from datetime import datetime, tzinfo
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.vertex_ai.common_utils import get_supports_system_message
from litellm.llms.vertex_ai.context_caching import transformation
from litellm.llms.vertex_ai.context_caching.transformation import (
    cached_messages_end_on_supported_turn,
    separate_cached_messages,
    transform_openai_messages_to_gemini_context_caching,
)
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue

litellm.suppress_debug_info = True

Provider: TypeAlias = Literal["vertex_ai", "vertex_ai_beta", "gemini"]
JsonObject: TypeAlias = dict[str, JsonValue]


class Case(TypedDict):
    model: str
    provider: Provider
    supports_system_message: bool
    cache_key: str
    project: str
    location: str
    now: str
    disable_transform: bool
    messages: list[JsonObject]


class Input(TypedDict):
    cases: list[Case]


class Result(TypedDict):
    cached: list[JsonObject]
    non_cached: list[JsonObject]
    eligible: bool
    system_supported: bool
    body: JsonObject | None


class Output(TypedDict):
    results: list[Result]


class FixedClock:
    def __init__(self, moment: datetime) -> None:
        self.moment = moment

    def now(self, tz: tzinfo | None = None) -> datetime:
        if tz is None or tz == self.moment.tzinfo:
            return self.moment
        return self.moment.astimezone(tz)


def case_result(case: Case) -> Result:
    transformation.datetime = FixedClock(datetime.fromisoformat(case["now"]))  # type: ignore[attr-defined,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/context_caching/transformation.py#L4
    litellm.disable_anthropic_gemini_context_caching_transform = case["disable_transform"]
    typed_messages = cast("list[AllMessageValues]", case["messages"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    cached, non_cached = separate_cached_messages(messages=typed_messages)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    cached_view = cast("list[JsonObject]", cached)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    non_cached_view = cast("list[JsonObject]", non_cached)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    supported_turn = cached_messages_end_on_supported_turn(cached)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    eligible = len(cached_view) > 0 and supported_turn
    body: JsonObject | None = None
    if eligible:
        request_body = transform_openai_messages_to_gemini_context_caching(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            model=case["model"],
            messages=list(cached),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            custom_llm_provider=case["provider"],
            cache_key=case["cache_key"],
            vertex_project=case["project"],
            vertex_location=case["location"],
        )
        body = cast("JsonObject", request_body)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    return {
        "cached": cached_view,
        "non_cached": non_cached_view,
        "eligible": eligible,
        "system_supported": get_supports_system_message(case["model"], case["provider"]),
        "body": body,
    }


def run(input_value: Input) -> Output:
    return {
        "results": [case_result(case) for case in input_value["cases"]],
    }
