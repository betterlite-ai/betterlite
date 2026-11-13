import os

# LiteLLM reads LITELLM_RUST per call; false pins the Python tokenizer the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

from typing import NoReturn, Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm._logging import verbose_logger
from litellm.llms.anthropic.pass_through.context_management import constants
from litellm.llms.anthropic.pass_through.context_management.editors.clear_tool_uses import (
    ClearToolUsesEditSpec,
    apply_clear_tool_uses_20250919,
)
from litellm.llms.anthropic.pass_through.context_management.errors import (
    AnthropicContextManagementError,
)
from litellm.llms.anthropic.pass_through.context_management.result import PolyfillResult
from litellm.types.llms.anthropic import AppliedEdit, CompactionBlock

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenCounterCall(Protocol):
    def __call__(
        self,
        *,
        model: str = ...,
        messages: list[dict[str, Json]] | None = ...,
        tools: list[dict[str, Json]] | None = ...,
    ) -> int: ...


class WarningCapture:
    def __init__(self, logs: list[str]) -> None:
        self._logs = logs

    def __call__(self, message: str, *args: object, **_kwargs: object) -> None:
        self._logs.append(message % args if args else message)


class ScriptedCounter:
    def __init__(self, values: list[int], calls: list[Json]) -> None:
        self._values = list(reversed(values))
        self._calls = calls

    def __call__(
        self,
        *,
        model: str,
        messages: list[dict[str, Json]] | None = None,
        tools: list[dict[str, Json]] | None = None,
        **_kwargs: object,
    ) -> int:
        if not self._values:
            raise ValueError("scripted token counts exhausted")
        value = self._values.pop()
        self._calls.append(
            {
                "model": model,
                "messages": cast("Json", messages),
                "tools": cast("Json", tools),
                "count": value,
            }
        )
        return value


class RealCounter:
    def __init__(self, calls: list[Json]) -> None:
        self._calls = calls
        self._counter = cast("TokenCounterCall", litellm.token_counter)

    def __call__(
        self,
        *,
        model: str,
        messages: list[dict[str, Json]] | None = None,
        tools: list[dict[str, Json]] | None = None,
        **_kwargs: object,
    ) -> int:
        value = self._counter(model=model, messages=messages, tools=tools)
        self._calls.append(
            {
                "model": model,
                "messages": cast("Json", messages),
                "tools": cast("Json", tools),
                "count": value,
            }
        )
        return value


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _integer(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def _messages(value: Json, name: str) -> list[dict[str, Json]]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [_object(entry, f"{name} entry") for entry in value]


def _optional_messages(value: Json, name: str) -> list[dict[str, Json]] | None:
    if value is None:
        return None
    return _messages(value, name)


def _scripted_values(value: Json) -> list[int]:
    if not isinstance(value, list):
        raise ValueError("values must be a list")
    return [_integer(entry, "values entry") for entry in value]


def _counter(input_value: dict[str, Json], calls: list[Json]) -> object:
    mode = _string(input_value.get("mode"), "mode")
    if mode == "real":
        return RealCounter(calls)
    if mode == "scripted":
        return ScriptedCounter(_scripted_values(input_value.get("values")), calls)
    raise ValueError(f"unknown counter mode: {mode}")


def _edit_spec(value: Json) -> ClearToolUsesEditSpec:
    return cast("ClearToolUsesEditSpec", _object(value, "edit_spec"))


def _applied_edits(value: Json) -> list[AppliedEdit]:
    if value is None:
        return []
    return cast("list[AppliedEdit]", _messages(value, "applied_edits"))


def _compaction_block(value: Json) -> CompactionBlock | None:
    if value is None:
        return None
    return cast("CompactionBlock", _object(value, "compaction_block"))


def editor(input_value: dict[str, Json]) -> Json:
    model = _string(input_value.get("model"), "model")
    messages = _messages(input_value.get("messages"), "messages")
    tools = _optional_messages(input_value.get("tools"), "tools")
    system = input_value.get("system")
    calls: list[Json] = []
    counter = _counter(_object(input_value.get("counter"), "counter"), calls)
    logs: list[str] = []
    with (
        patch.object(litellm, "token_counter", counter),
        patch.object(verbose_logger, "warning", WarningCapture(logs)),
    ):
        edited, applied = apply_clear_tool_uses_20250919(
            model=model,
            messages=cast("list[dict[str, object]]", messages),
            tools=cast("list[dict[str, object]] | None", tools),
            system=cast("str | list[dict[str, object]] | None", system),
            edit_spec=_edit_spec(input_value.get("edit_spec")),
        )
    result: dict[str, Json] = {
        "messages": cast("Json", edited),
        "applied": cast("Json", applied),
        "warnings": cast("Json", logs),
        "calls": calls,
    }
    return result


def _raise_management_error(value: Json) -> NoReturn:
    error = _object(value, "error")
    raise AnthropicContextManagementError(
        status_code=_integer(error.get("status_code"), "status_code"),
        message=_string(error.get("message"), "message"),
    )


def visible_edits(input_value: dict[str, Json]) -> Json:
    error = input_value.get("error")
    if error is not None:
        _raise_management_error(error)
    result = PolyfillResult(
        messages=[],
        system=None,
        applied_edits=_applied_edits(input_value.get("applied_edits")),
        compaction_block=_compaction_block(input_value.get("compaction_block")),
    )
    visible: dict[str, Json] = {
        "visible": cast("Json", result.applied_edits_for_response())
    }
    return visible


def constants_value() -> Json:
    value: dict[str, Json] = {
        "clear_tool_uses_edit_type": constants.CLEAR_TOOL_USES_EDIT_TYPE,
        "default_input_tokens_trigger": constants.DEFAULT_INPUT_TOKENS_TRIGGER,
        "default_keep_tool_uses": constants.DEFAULT_KEEP_TOOL_USES,
        "cleared_tool_result_content": constants.CLEARED_TOOL_RESULT_PLACEHOLDER,
        "compact_edit_type": constants.COMPACT_EDIT_TYPE,
        "compact_default_trigger_tokens": constants.COMPACT_DEFAULT_TRIGGER_TOKENS,
        "compact_min_trigger_tokens": constants.COMPACT_MIN_TRIGGER_TOKENS,
        "compact_summary_max_tokens": constants.COMPACT_SUMMARY_MAX_TOKENS,
        "summary_max_tokens_key": constants.COMPACT_SUMMARY_MAX_TOKENS_SETTING_KEY,
        "summary_timeout_seconds": constants.COMPACT_SUMMARY_TIMEOUT_SECONDS,
        "summary_model_key": constants.COMPACT_SUMMARY_MODEL_SETTING_KEY,
        "compact_summary_system_prefix": constants.COMPACT_SUMMARY_SYSTEM_PREFIX,
        "compact_default_instructions": constants.COMPACT_DEFAULT_INSTRUCTIONS,
        "compact_no_tool_calls_suffix": constants.COMPACT_NO_TOOL_CALLS_SUFFIX,
    }
    return value


def run(input_value: dict[str, Json]) -> Json:
    action = _string(input_value.get("action"), "action")
    if action == "editor":
        return editor(input_value)
    if action == "visible_edits":
        return visible_edits(input_value)
    if action == "constants":
        return constants_value()
    raise ValueError(f"unsupported action {action}")
