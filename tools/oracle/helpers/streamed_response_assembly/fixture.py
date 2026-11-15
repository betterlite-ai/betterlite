from datetime import UTC, datetime
from typing import Final, Literal

from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue as Json

from .specs import LoggingInput

LOG_MODEL: Final = "gpt-4o"
START_TIME: Final = datetime(2024, 1, 1, tzinfo=UTC)


class FixedPrompt:
    def __init__(self, count: int) -> None:
        self.count = count

    def __call__(self) -> int:
        return self.count


class FailingPrompt:
    def __call__(self) -> int:
        raise ValueError("count_prompt_tokens fixture failure")


def prompt_callback(prompt: int | Literal["raise"] | None) -> FixedPrompt | FailingPrompt | None:
    match prompt:
        case None:
            return None
        case "raise":
            return FailingPrompt()
        case int() as count:
            return FixedPrompt(count)


def logging_fixture(fixture: LoggingInput | None) -> Logging | None:
    if fixture is None:
        return None
    log = Logging(
        model=LOG_MODEL,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=START_TIME,
        litellm_call_id="streamed-response-assembly",
        function_id="streamed-response-assembly",
    )
    log.model_call_details = model_call_details(fixture)
    optional_params: dict[str, Json] = {}
    log.optional_params = optional_params
    return log


def model_call_details(fixture: LoggingInput) -> dict[str, Json]:
    return {
        "custom_llm_provider": fixture["provider"],
        "cache_hit": fixture["cache_hit"],
    }
