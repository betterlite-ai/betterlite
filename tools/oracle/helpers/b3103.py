import json
from datetime import date
from typing import ClassVar, Final, Literal, TypeAlias, TypedDict, cast

from litellm.proxy.management_endpoints.usage_endpoints import ai_usage_chat
from litellm.proxy.management_endpoints.usage_endpoints.ai_usage_chat import get_tools_for_role
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
TOOL_NAMES: Final[dict[str, str]] = {
    "usage": "get_usage_data",
    "team": "get_team_usage_data",
    "tag": "get_tag_usage_data",
}


class MetricsInput(TypedDict, total=False):
    spend: float
    api_requests: int
    total_tokens: int


class EntryInput(TypedDict, total=False):
    metrics: MetricsInput
    metadata: dict[str, str]


class BreakdownInput(TypedDict, total=False):
    models: dict[str, EntryInput]
    providers: dict[str, EntryInput]
    entities: dict[str, EntryInput]


class DayInput(TypedDict, total=False):
    date: str
    metrics: MetricsInput
    breakdown: BreakdownInput


class MetadataInput(TypedDict, total=False):
    total_spend: float
    total_api_requests: int
    total_successful_requests: int
    total_failed_requests: int
    total_tokens: int


class PageInput(TypedDict):
    results: list[DayInput]
    metadata: MetadataInput


class ToolsCall(TypedDict):
    function: Literal["tools"]
    is_admin: bool


class PromptCall(TypedDict):
    function: Literal["prompt"]
    is_admin: bool
    today: str


class CsvIdsCall(TypedDict):
    function: Literal["csv_ids"]
    raw: str | None


class FetchArgsCall(TypedDict):
    function: Literal["fetch_args"]
    tool: Literal["usage", "team", "tag"]
    arguments: dict[str, str]
    user_id: str | None
    is_admin: bool


class SummaryUsageCall(TypedDict):
    function: Literal["summary_usage"]
    page: PageInput


class SummaryEntityCall(TypedDict):
    function: Literal["summary_entity"]
    page: PageInput
    label: Literal["team", "tag"]


class ToolCallEvent(TypedDict):
    type: Literal["tool_call"]
    tool_name: str
    tool_label: str
    arguments: dict[str, str]
    status: Literal["running", "complete", "error"]


class ChunkEvent(TypedDict):
    type: Literal["chunk"]
    content: str


class SseCall(TypedDict):
    function: Literal["sse"]
    event: ToolCallEvent | ChunkEvent


Input: TypeAlias = (
    ToolsCall
    | PromptCall
    | CsvIdsCall
    | FetchArgsCall
    | SummaryUsageCall
    | SummaryEntityCall
    | SseCall
)


class FixedDate(date):
    iso: ClassVar[str] = "1970-01-01"

    @classmethod
    def today(cls) -> date:
        return date.fromisoformat(cls.iso)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "tools":
        tools = get_tools_for_role(input_value["is_admin"])
        return {"tools": JSON.validate_json(json.dumps(tools))}
    if input_value["function"] == "prompt":
        return {"prompt": prompt_value(input_value)}
    if input_value["function"] == "csv_ids":
        return {"ids": csv_ids_value(input_value["raw"])}
    if input_value["function"] == "fetch_args":
        return {"kwargs": fetch_kwargs_value(input_value)}
    if input_value["function"] == "sse":
        return {"frame": sse_frame_value(input_value["event"])}
    return {"summary": page_summary(input_value)}


def prompt_value(call: PromptCall) -> str:
    FixedDate.iso = call["today"]
    ai_usage_chat.date = FixedDate
    return ai_usage_chat._build_system_prompt(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L233-L236
        call["is_admin"]
    )


def csv_ids_value(raw: str | None) -> JsonValue:
    ids = ai_usage_chat._parse_csv_ids(raw)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L247-L250
    if ids is None:
        return None
    return [*ids]


def fetch_kwargs_value(call: FetchArgsCall) -> JsonValue:
    kwargs = ai_usage_chat._resolve_fetch_kwargs(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L460-L490
        TOOL_NAMES[call["tool"]],
        call["arguments"],
        call["user_id"],
        call["is_admin"],
    )
    rendered: dict[str, JsonValue] = {}
    for key, value in kwargs.items():
        rendered[key] = value
    return rendered


def page_summary(call: SummaryUsageCall | SummaryEntityCall) -> str:
    data = cast("ai_usage_chat._UsageDump", call["page"])  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L97-L102
    if call["function"] == "summary_usage":
        return ai_usage_chat._summarise_usage_data(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L371-L400
            data
        )
    label = "Team" if call["label"] == "team" else "Tag"
    return ai_usage_chat._summarise_entity_data(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L403-L425
        data, label
    )


def sse_frame_value(event: ToolCallEvent | ChunkEvent) -> str:
    return ai_usage_chat._sse(event)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/usage_endpoints/ai_usage_chat.py#L456-L457
