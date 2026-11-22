from collections.abc import Awaitable
from pathlib import Path
from typing import Final, NoReturn, Protocol, runtime_checkable

from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite._completion.arguments import JSON_OBJECT, CallValue, config_path
from betterlite._completion.classes import _ACOMPLETION, _COMPLETION
from betterlite._completion.stream import CompletionOutcome, CustomStreamWrapper
from betterlite.types.utils import ModelResponse

CONFIG: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


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


def _missing_adapter(adapter_id: str) -> NoReturn:
    adapters = betterlite.adapters
    raise ValueError(
        f"No matching adapter given. Received 'adapter_id'={adapter_id}, "
        f"litellm.adapters={adapters}"
    )


def _find_adapter(adapter_id: str) -> CompletionAdapter:
    # litellm.adapters is a list of {"id": ..., "adapter": ...}; the last match wins as in LiteLLM
    found: CompletionAdapter | None = None
    for item in betterlite.adapters:
        match item:
            case dict():
                if item.get("id") != adapter_id:
                    continue
                candidate = item.get("adapter")
                match candidate:
                    case CompletionAdapter():
                        found = candidate
                    case object():
                        continue
            case object():
                continue
    if found is None:
        _missing_adapter(adapter_id)
    return found


def _translated(
    adapter: CompletionAdapter,
    response: CompletionOutcome | Awaitable[CompletionOutcome],
) -> CompletionOutcome | Awaitable[CompletionOutcome] | None:
    match response:
        case ModelResponse():
            return adapter.translate_completion_output_params(response=response)
        case CustomStreamWrapper():
            return adapter.translate_completion_output_params_streaming(
                completion_stream=response
            )
        case object():
            return None


def adapter_completion(
    *, adapter_id: str, **kwargs: CallValue
) -> CompletionOutcome | Awaitable[CompletionOutcome] | None:
    adapter = _find_adapter(adapter_id)
    configured = JSON_OBJECT.validate_python(dict(kwargs))
    new_kwargs = adapter.translate_completion_input_params(kwargs=configured)
    return _translated(adapter, _COMPLETION(**new_kwargs))


async def aadapter_completion(
    *, adapter_id: str, **kwargs: CallValue
) -> CompletionOutcome | None:
    adapter = _find_adapter(adapter_id)
    configured = JSON_OBJECT.validate_python(dict(kwargs))
    new_kwargs = adapter.translate_completion_input_params(kwargs=configured)
    translated = _translated(adapter, await _ACOMPLETION(**new_kwargs))
    if isinstance(translated, Awaitable):
        return await translated
    return translated


def config_completion(
    **kwargs: CallValue,
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    configured: object = betterlite.config_path
    if configured is None:
        configured = config_path
    if configured is None:
        raise ValueError(
            "No config path set, please set a config path using "
            "`litellm.config_path = 'path/to/config.json'`"
        )
    match configured:
        case str():
            config = CONFIG.validate_json(Path(configured).read_text())
            merged: dict[str, CallValue] = {**kwargs, **config}
            return _COMPLETION(**merged)
        case object():
            raise ValueError("betterlite.config_path must be a string")
