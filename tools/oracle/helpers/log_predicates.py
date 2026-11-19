from datetime import datetime, timezone
from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.spend_tracking import spend_management_endpoints
from pydantic import JsonValue, TypeAdapter

STATUS_CONDITION: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class SearchInput(TypedDict):
    function: Literal["search"]
    search: str
    start: str
    end: str
    next_param_index: int


class StatusInput(TypedDict):
    function: Literal["status"]
    status_filter: str | None


class SpanInput(TypedDict):
    function: Literal["span"]
    span_type: str | None


Input: TypeAlias = SearchInput | StatusInput | SpanInput


def search_value(call: SearchInput) -> JsonValue:
    start = datetime.fromisoformat(call["start"].strip()).replace(tzinfo=timezone.utc)
    end = datetime.fromisoformat(call["end"].strip()).replace(tzinfo=timezone.utc)
    condition = spend_management_endpoints._build_spend_log_search_condition(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_management_endpoints.py#L2438
        call["search"], start, end, call["next_param_index"]
    )
    params: list[JsonValue] = [
        call["search"],
        start.isoformat(),
        end.isoformat(),
    ]
    return {"sql": condition.sql, "params": params}


def status_value(call: StatusInput) -> JsonValue:
    condition = spend_management_endpoints._build_status_filter_condition(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_management_endpoints.py#L4810
        call["status_filter"]
    )
    return STATUS_CONDITION.validate_python(condition)


def span_value(call: SpanInput) -> JsonValue:
    return spend_management_endpoints._span_type_sql_condition(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_management_endpoints.py#L4829
        call["span_type"]
    )


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "search":
            return search_value(input_value)
        case "status":
            return status_value(input_value)
        case "span":
            return span_value(input_value)
