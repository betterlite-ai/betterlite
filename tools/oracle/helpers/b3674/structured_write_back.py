from collections.abc import Callable, Mapping, Sequence
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.llms.anthropic.chat.guardrail_translation.handler import AnthropicMessagesHandler
from pydantic import JsonValue


class DataScenario(TypedDict):
    data: dict[str, JsonValue]


class SystemConversionScenario(TypedDict):
    message: dict[str, JsonValue]


class FoldScenario(TypedDict):
    data: dict[str, JsonValue]
    leading: list[JsonValue]
    include_existing: bool


class DeferScenario(TypedDict):
    messages: list[JsonValue]


class StructuredScenario(TypedDict):
    data: dict[str, JsonValue]
    messages: list[JsonValue]
    preserve_system_messages: bool
    compute_hoisted: bool


class HoistedRequest(TypedDict):
    function: Literal["hoisted"]
    scenarios: list[DataScenario]


class SystemConversionRequest(TypedDict):
    function: Literal["system_conversion"]
    scenarios: list[SystemConversionScenario]


class FoldRequest(TypedDict):
    function: Literal["fold"]
    scenarios: list[FoldScenario]


class DeferRequest(TypedDict):
    function: Literal["defer"]
    scenarios: list[DeferScenario]


class StructuredRequest(TypedDict):
    function: Literal["structured"]
    scenarios: list[StructuredScenario]


ReadHoisted: TypeAlias = Callable[[Mapping[str, JsonValue]], dict[str, JsonValue] | None]
ConvertSystemRow: TypeAlias = Callable[[Mapping[str, JsonValue]], dict[str, JsonValue] | None]
FoldLeading: TypeAlias = Callable[[dict[str, JsonValue], Sequence[JsonValue], bool], None]
DeferSystems: TypeAlias = Callable[[Sequence[JsonValue]], list[JsonValue]]
WriteStructured: TypeAlias = Callable[
    [dict[str, JsonValue], Sequence[JsonValue], Mapping[str, JsonValue] | None, bool],
    None,
]


def hoisted_reader() -> ReadHoisted:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    return cast(ReadHoisted, handler._hoisted_top_level_system_message)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L686-L699


def hoisted_answer(scenario: DataScenario) -> dict[str, JsonValue]:
    message = hoisted_reader()(scenario["data"])
    return {"message": message}


def system_conversion_answer(scenario: SystemConversionScenario) -> dict[str, JsonValue]:
    convert = cast(ConvertSystemRow, AnthropicMessagesHandler._openai_system_message_to_anthropic)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L714-L739
    row = convert(scenario["message"])
    return {"row": row}


def fold_answer(scenario: FoldScenario) -> dict[str, JsonValue]:
    fold = cast(FoldLeading, AnthropicMessagesHandler._fold_leading_systems_into_top_level)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L741-L772
    fold(scenario["data"], scenario["leading"], scenario["include_existing"])
    return {"data": scenario["data"]}


def defer_answer(scenario: DeferScenario) -> dict[str, JsonValue]:
    defer = cast(DeferSystems, AnthropicMessagesHandler._defer_systems_inside_tool_exchanges)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L790-L820
    return {"messages": defer(scenario["messages"])}


def structured_answer(scenario: StructuredScenario) -> dict[str, JsonValue]:
    data = scenario["data"]
    hoisted = hoisted_reader()(data) if scenario["compute_hoisted"] else None
    write = cast(WriteStructured, AnthropicMessagesHandler._write_back_structured_messages)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L822-L891
    write(data, scenario["messages"], hoisted, scenario["preserve_system_messages"])
    return {"data": data}
