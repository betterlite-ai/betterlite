from datetime import UTC, datetime, tzinfo
from typing import Final, Literal, Self, TypedDict

from litellm.proxy.management_endpoints import key_management_endpoints
from litellm.proxy.management_endpoints.key_management_endpoints import (
    _build_key_filter_conditions,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6903-L7037
    _build_status_where_clause,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6885-L6890
    _derive_key_status,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6476-L6483
    _validate_sort_params,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6843-L6872
)
from pydantic import JsonValue, TypeAdapter
from pydantic_core import to_json
from typing_extensions import NotRequired, override

_JSON: Final = TypeAdapter(JsonValue)


# the case supplies the request time, so the builder's datetime.now has one answer
class FrozenClock(datetime):
    fixed: datetime = datetime(1970, 1, 1, tzinfo=UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        moment = cls.fixed if tz is None else cls.fixed.astimezone(tz)
        return cls.fromtimestamp(moment.timestamp(), tz=moment.tzinfo)


class StatusCall(TypedDict):
    function: Literal["status"]
    blocked: bool | None
    expires: str | None
    now: str


class StatusClauseCall(TypedDict):
    function: Literal["status_clause"]
    status_filter: str | None
    now: str


class SortCall(TypedDict):
    function: Literal["sort"]
    sort_by: str | None
    sort_order: str


class FilterArgs(TypedDict):
    user_id: str | None
    team_id: str | None
    organization_id: str | None
    key_alias: str | None
    key_hash: str | None
    exclude_team_id: str | None
    admin_team_ids: list[str] | None
    member_team_ids: list[str] | None
    include_created_by_keys: NotRequired[bool]
    project_id: NotRequired[str | None]
    access_group_id: NotRequired[str | None]
    agent_id: NotRequired[str | None]
    use_substring_matching: NotRequired[bool]
    use_key_alias_substring_matching: NotRequired[bool]
    expires_filter: NotRequired[str | None]
    search: NotRequired[str | None]
    status_filter: NotRequired[str | None]


class FiltersCall(TypedDict):
    function: Literal["filters"]
    now: str
    filters: FilterArgs


Call = StatusCall | StatusClauseCall | SortCall | FiltersCall


def _status_answer(call: StatusCall) -> JsonValue:
    row: dict[str, JsonValue] = {"blocked": call["blocked"], "expires": call["expires"]}
    status: JsonValue = _derive_key_status(row, datetime.fromisoformat(call["now"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6476-L6483
    return status


def _status_clause_answer(call: StatusClauseCall) -> JsonValue:
    where = _build_status_where_clause(call["status_filter"], datetime.fromisoformat(call["now"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6885-L6890
    if where is None:
        return None
    return _JSON.validate_json(to_json(where))


def _sort_answer(call: SortCall) -> JsonValue:
    order_by = _validate_sort_params(call["sort_by"], call["sort_order"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6843-L6872
    if order_by is None:
        return None
    return _JSON.validate_json(to_json(order_by))


def _filters_answer(call: FiltersCall) -> JsonValue:
    FrozenClock.fixed = datetime.fromisoformat(call["now"])
    key_management_endpoints.datetime = FrozenClock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6903-L7037
    where = _build_key_filter_conditions(**call["filters"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6903-L7037
    return _JSON.validate_json(to_json(where))


def run(input_value: Call) -> JsonValue:
    result: JsonValue = None
    match input_value["function"]:
        case "status":
            result = _status_answer(input_value)
        case "status_clause":
            result = _status_clause_answer(input_value)
        case "sort":
            result = _sort_answer(input_value)
        case "filters":
            result = _filters_answer(input_value)
    return result
