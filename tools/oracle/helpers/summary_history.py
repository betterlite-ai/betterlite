import logging
from logging.handlers import MemoryHandler
from types import TracebackType
from typing import Final, Literal, NoReturn, Self, TypeAlias, TypedDict, TypeVar, cast

import litellm
from litellm.llms.anthropic.pass_through.adapters import transformation
from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from litellm.llms.anthropic.pass_through.context_management.editors.compact import (
    _augment_system_with_summary,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L701-L718
    _build_summary_messages,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L946-L1016
    _build_summary_prompt,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L755-L762
    _count_effective_tokens,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L785-L874
    _extract_summary_text,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L915-L922
    _propagate_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L765-L782
    _resolve_trigger_tokens,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L721-L752
    _select_last_user_question,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L877-L912
    _slice_around_compaction_block,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L630-L676
    _strip_compaction_blocks,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L679-L698
)
from litellm.llms.anthropic.pass_through.context_management.editors.compact import (
    apply_client_compaction_block_history as client_history_call,
)
from litellm.llms.anthropic.pass_through.context_management.result import PolyfillResult
from litellm.types.llms.anthropic import CompactionBlock
from pydantic import JsonValue, TypeAdapter

ANSWER: Final = TypeAdapter(JsonValue)
POLYFILL: Final = TypeAdapter(PolyfillResult)
LOGGER: Final = logging.getLogger("LiteLLM")
BROKEN: Final = "broken"
DIAGNOSTIC: Final = "compact_20260112:"

Outcome: TypeAlias = Literal["translated", "broken"]
Argument = TypeVar("Argument")


class SlicingCall(TypedDict):
    action: Literal["slicing"]
    messages: list[dict[str, JsonValue]]


class TriggerCall(TypedDict):
    action: Literal["trigger"]
    edit_spec: dict[str, JsonValue]


class PromptsCall(TypedDict):
    action: Literal["prompts"]
    edit_spec: dict[str, JsonValue]
    tools: list[dict[str, JsonValue]] | None
    system: str | list[dict[str, JsonValue]] | None
    summary: str | None
    messages: list[dict[str, JsonValue]]
    translation: Outcome


class MetadataCall(TypedDict):
    action: Literal["metadata"]
    parent: dict[str, JsonValue] | None


class ScriptedMode(TypedDict):
    mode: Literal["scripted"]
    values: list[int]


class RealMode(TypedDict):
    mode: Literal["real"]


class CountCall(TypedDict):
    action: Literal["count"]
    model: str
    messages: list[dict[str, JsonValue]]
    compaction_block: CompactionBlock | None
    tools: list[dict[str, JsonValue]] | None
    system: str | list[dict[str, JsonValue]] | None
    counter: ScriptedMode | RealMode
    translation: Outcome


class ExtractionCall(TypedDict):
    action: Literal["extraction"]
    raw: str | None


class ClientHistoryCall(TypedDict):
    action: Literal["client_history"]
    messages: list[dict[str, JsonValue]]
    system: str | list[dict[str, JsonValue]] | None


Call: TypeAlias = (
    SlicingCall
    | TriggerCall
    | PromptsCall
    | MetadataCall
    | CountCall
    | ExtractionCall
    | ClientHistoryCall
)


class TranslationError(ValueError):
    def refuse(self, *_args: Argument, **_kwargs: Argument) -> NoReturn:
        raise self


# compact.py builds the adapter inside each counting and summary call, so a case swaps the class
class BrokenAdapter(MessagesAdapter):
    def __init__(self) -> None:
        refuse = TranslationError(BROKEN).refuse
        self.translate_anthropic_messages_to_openai = refuse  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L807-L813
        self.translate_anthropic_tools_to_openai = refuse  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L828-L831


class LogCapture(MemoryHandler):
    def __init__(self) -> None:
        super().__init__(capacity=1000, flushLevel=logging.CRITICAL)
        self.previous = LOGGER.level

    def __enter__(self) -> Self:
        LOGGER.addHandler(self)
        LOGGER.setLevel(logging.DEBUG)
        return self

    def __exit__(
        self,
        _kind: type[BaseException] | None,
        _error: BaseException | None,
        _trace: TracebackType | None,
    ) -> None:
        LOGGER.removeHandler(self)
        LOGGER.setLevel(self.previous)

    def lines(self) -> list[JsonValue]:
        return [
            {"level": record.levelname.lower(), "message": record.getMessage()}
            for record in self.buffer
            if record.getMessage().startswith(DIAGNOSTIC)
        ]


class ScriptedCounter:
    def __init__(self, values: list[int]) -> None:
        self.values = [*reversed(values)]
        self.calls: list[JsonValue] = []

    def __call__(
        self,
        *,
        model: str,
        messages: list[JsonValue] | None = None,
        tools: list[JsonValue] | None = None,
        text: str | None = None,
    ) -> int:
        if not self.values:
            raise ValueError("scripted token counts exhausted")
        counted = self.values.pop()
        self.calls.append(
            {"model": model, "messages": messages, "tools": tools, "text": text, "count": counted},
        )
        return counted


def slicing(call: SlicingCall) -> JsonValue:
    messages = call["messages"]
    history = cast("list[dict[str, object]]", messages)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L679-L681
    sliced, block = _slice_around_compaction_block(messages)
    stripped = _strip_compaction_blocks(history)
    question = _select_last_user_question(history)[0]
    answer = {
        "messages": sliced,
        "block": block,
        "stripped": stripped,
        "last_question": question,
        "input": messages,
    }
    return ANSWER.validate_python(answer)


def trigger(call: TriggerCall) -> JsonValue:
    tokens, warnings = _resolve_trigger_tokens(call["edit_spec"])
    answer: dict[str, JsonValue] = {"tokens": tokens, "warnings": ANSWER.validate_python(warnings)}
    return answer


def prompts(call: PromptsCall) -> JsonValue:
    system = cast("str | list[dict[str, object]] | None", call["system"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L946-L950
    history = cast("list[dict[str, object]]", call["messages"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L946-L950
    prompt = _build_summary_prompt(call["edit_spec"], call["tools"])
    summary = call["summary"]
    augmented = None if summary is None else _augment_system_with_summary(system, summary)
    if call["translation"] == "broken":
        transformation.LiteLLMAnthropicMessagesAdapter = BrokenAdapter  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L958-L964
    with LogCapture() as capture:
        messages = _build_summary_messages(history, prompt, system=system)
    answer: dict[str, JsonValue] = {
        "prompt": prompt,
        "augmented_system": ANSWER.validate_python(augmented),
        "summary_messages": ANSWER.validate_python(messages),
        "logs": capture.lines(),
    }
    return answer


def count(call: CountCall) -> JsonValue:
    mode = call["counter"]
    scripted = ScriptedCounter(mode["values"]) if mode["mode"] == "scripted" else None
    if scripted is not None:
        litellm.token_counter = scripted
    if call["translation"] == "broken":
        transformation.LiteLLMAnthropicMessagesAdapter = BrokenAdapter  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L800-L806
    system = cast("str | list[dict[str, object]] | None", call["system"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L785-L791
    history = cast("list[dict[str, object]]", call["messages"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L785-L791
    tools = cast("list[dict[str, object]] | None", call["tools"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L785-L791
    with LogCapture() as capture:
        total = _count_effective_tokens(
            call["model"], history, call["compaction_block"], tools, system,
        )
    answer: dict[str, JsonValue] = {
        "total": total,
        "calls": [] if scripted is None else scripted.calls,
        "logs": capture.lines(),
    }
    return answer


def client_history(call: ClientHistoryCall) -> JsonValue:
    system = cast("str | list[dict[str, object]] | None", call["system"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1114-L1118
    history = cast("list[dict[str, object]]", call["messages"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1114-L1118
    with LogCapture() as capture:
        result = client_history_call(messages=history, system=system)
    if result is None:
        return {"result": None, "logs": capture.lines()}
    return {"result": ANSWER.validate_json(POLYFILL.dump_json(result)), "logs": capture.lines()}


def run(input_value: Call) -> JsonValue:
    answer: JsonValue
    if input_value["action"] == "slicing":
        answer = slicing(input_value)
    elif input_value["action"] == "trigger":
        answer = trigger(input_value)
    elif input_value["action"] == "prompts":
        answer = prompts(input_value)
    elif input_value["action"] == "metadata":
        answer = {"metadata": ANSWER.validate_python(_propagate_metadata(input_value["parent"]))}
    elif input_value["action"] == "count":
        answer = count(input_value)
    elif input_value["action"] == "extraction":
        answer = {"summary": _extract_summary_text(input_value["raw"])}
    else:
        answer = client_history(input_value)
    return answer
