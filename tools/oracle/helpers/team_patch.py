from datetime import UTC, datetime, tzinfo
from typing import Final, Self, TypedDict

from litellm.proxy._types import UpdateTeamRequest
from litellm.proxy.common_utils import timezone_utils
from litellm.proxy.management_endpoints.team_endpoints import _set_budget_reset_at  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L2645-L2663
from pydantic import JsonValue, TypeAdapter
from pydantic_core import to_json
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)
_DUMP: Final = TypeAdapter(dict[str, JsonValue])
_TEXT: Final = TypeAdapter(str)


# the case supplies the request time, so every LiteLLM datetime.now in the reset policy has one answer
class FrozenClock(datetime):
    fixed: datetime = datetime(1970, 1, 1, tzinfo=UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        moment = cls.fixed if tz is None else cls.fixed.astimezone(tz)
        return cls.fromtimestamp(moment.timestamp(), tz=moment.tzinfo)


class ResetCall(TypedDict):
    request: dict[str, JsonValue]
    now: str


class Input(TypedDict):
    calls: list[ResetCall]


def _reset_answer(call: ResetCall) -> JsonValue:
    data = UpdateTeamRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L2219
    updated: dict[str, JsonValue] = _DUMP.validate_json(data.model_dump_json(exclude_unset=True))
    FrozenClock.fixed = datetime.fromisoformat(call["now"])
    timezone_utils.datetime = FrozenClock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/timezone_utils.py#L64-L68
    _set_budget_reset_at(data, updated)
    if data.budget_limits:
        updated["budget_limits"] = _JSON.validate_json(_TEXT.validate_python(updated["budget_limits"]))
    return _JSON.validate_json(to_json(updated))


def _outcome(call: ResetCall) -> JsonValue:
    try:
        value = _reset_answer(call)
    except (ValueError, TypeError, KeyError, OverflowError):
        return {"ok": False}
    return {"ok": True, "value": value}


def run(input_value: Input) -> JsonValue:
    outcomes: list[JsonValue] = [_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
