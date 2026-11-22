from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, TypedDict, runtime_checkable

from pydantic import JsonValue

import betterlite
from betterlite._completion.arguments import JSON, JSON_OBJECT, CallValue, config_path
from betterlite._completion.completion import completion_from_arguments, settle_outcome
from betterlite._completion.stream import CompletionOutcome, CustomStreamWrapper
from betterlite.types.utils import ModelResponse


@runtime_checkable
class CompletionAdapter(Protocol):
    def translate_completion_input_params(  # ast-grep-ignore: no-long-names
        self, *, kwargs: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...

    def translate_completion_output_params(  # ast-grep-ignore: no-long-names
        self, *, response: ModelResponse
    ) -> ModelResponse: ...

    def translate_completion_output_params_streaming(  # ast-grep-ignore: no-long-names
        self, *, completion_stream: CustomStreamWrapper
    ) -> CompletionOutcome | None: ...


class AdapterEntry(TypedDict):
    id: str
    adapter: CompletionAdapter


# betterlite.adapters is JSON in the settings task, so this seam carries the Python adapters
adapter_entries: list[AdapterEntry] | None = None


def _find_adapter(adapter_id: str) -> CompletionAdapter:
    # litellm.adapters holds {"id": ..., "adapter": ...} entries; the last match wins as in LiteLLM
    entries = adapter_entries
    if entries is None:
        entries = []
    found: CompletionAdapter | None = None
    for entry in entries:
        if entry["id"] == adapter_id:
            found = entry["adapter"]
    if found is None:
        configured = betterlite.adapters
        raise ValueError(
            f"No matching adapter given. Received 'adapter_id'={adapter_id}, "
            f"litellm.adapters={configured}"
        )
    return found


# ast-grep-ignore: no-one-line-functions
def _json_values(values: dict[str, CallValue]) -> dict[str, JsonValue]:
    return {name: JSON.validate_python(value) for name, value in values.items()}


def _translated(
    adapter: CompletionAdapter,
    response: CompletionOutcome | Awaitable[CompletionOutcome],
) -> CompletionOutcome | None:
    match response:
        case ModelResponse():
            return adapter.translate_completion_output_params(response=response)
        case CustomStreamWrapper():
            return adapter.translate_completion_output_params_streaming(
                completion_stream=response
            )
        case Awaitable():
            return None


def adapter_completion(
    *, adapter_id: str, **kwargs: CallValue
) -> CompletionOutcome | None:
    adapter = _find_adapter(adapter_id)
    translated = adapter.translate_completion_input_params(kwargs=_json_values(kwargs))
    return _translated(adapter, completion_from_arguments(translated))


async def aadapter_completion(
    *, adapter_id: str, **kwargs: CallValue
) -> CompletionOutcome | None:
    adapter = _find_adapter(adapter_id)
    translated = adapter.translate_completion_input_params(kwargs=_json_values(kwargs))
    outcome = await settle_outcome(
        completion_from_arguments({**translated, "acompletion": True})
    )
    return _translated(adapter, outcome)


def config_completion(
    **kwargs: CallValue,
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    configured = config_path
    if configured is None:
        configured = betterlite.config_path
    if configured is None:
        raise ValueError(
            "No config path set, please set a config path using "
            "`litellm.config_path = 'path/to/config.json'`"
        )
    supplied = _json_values(kwargs)
    match configured:
        case str():
            config = JSON_OBJECT.validate_json(Path(configured).read_text(encoding="utf-8"))
            return completion_from_arguments({**supplied, **config})
        case dict():
            return completion_from_arguments({**supplied, **configured})
        case object():
            raise ValueError("betterlite.config_path must be a string")
