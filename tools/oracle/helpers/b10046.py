from datetime import UTC, datetime
from typing import Final, Literal, TypeAlias, TypedDict

import litellm.utils
from litellm.router_utils.add_retry_fallback_headers import apply_remaining_usage_headers
from litellm.router_utils.pre_call_checks import io_token_rate_limit_check
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)


class FixedClock:
    def __init__(self, moment: datetime) -> None:
        self.moment = moment

    def __call__(self) -> datetime:
        return self.moment


class BuildCall(TypedDict):
    function: Literal["build"]
    itpm: int | None
    otpm: int | None
    current_itpm: int | None
    current_otpm: int | None
    now: int


class ApplyCall(TypedDict):
    function: Literal["apply"]
    headers: dict[str, JsonValue]
    remaining: dict[str, int]


Input: TypeAlias = BuildCall | ApplyCall


def build_headers(call: BuildCall) -> JsonValue:
    moment = datetime.fromtimestamp(call["now"], tz=UTC)
    original = litellm.utils.get_utc_datetime
    io_token_rate_limit_check.get_utc_datetime = FixedClock(moment)  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/io_token_rate_limit_check.py#L26
    try:
        headers = io_token_rate_limit_check.build_io_token_rate_limit_headers(
            itpm_limit=call["itpm"],
            otpm_limit=call["otpm"],
            current_itpm=call["current_itpm"],
            current_otpm=call["current_otpm"],
        )
    finally:
        io_token_rate_limit_check.get_utc_datetime = original  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/io_token_rate_limit_check.py#L26
    return _JSON.validate_python(headers)


def apply_headers(call: ApplyCall) -> JsonValue:
    # LiteLLM merges into a dict[str, object]; the values stay JSON
    additional_headers: dict[str, object] = {}  # ast-grep-ignore: no-loose-types
    additional_headers.update(call["headers"])
    apply_remaining_usage_headers(additional_headers, call["remaining"])
    return _JSON.validate_python(additional_headers)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "build":
        return build_headers(input_value)
    return apply_headers(input_value)
