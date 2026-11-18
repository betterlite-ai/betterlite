import asyncio
import inspect
import subprocess
import sys
from collections.abc import Iterable
from typing import Final

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite._custom_routing import CustomRoutingStrategyBase

MODEL: Final = "gpt-4o"
RENAMED: Final = {"input": "input_value"}
KEYWORD_ONLY: Final = frozenset({"specific_deployment", "request_kwargs"})
ROW: Final = TypeAdapter(list[tuple[str, str, str]])
# LiteLLM types input and request_kwargs as bare list and dict, which strict mypy reads as Any,
# so the probe reads its signature in a subprocess and this file only sees JSON rows
LITELLM_PROBE: Final = """
import inspect
import json
import sys

from litellm.types.router import CustomRoutingStrategyBase

method = getattr(CustomRoutingStrategyBase, sys.argv[1])
rows: list[list[str]] = []
for parameter in inspect.signature(method).parameters.values():
    text = str(parameter)
    default = text.partition(" = " if " = " in text else "=")[2]
    rows.append([parameter.name, parameter.kind.name, default])
print(json.dumps(rows))
"""


def parameters(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    found: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        # str(Parameter) spells the default after " = ", or after "=" when it has no annotation
        text = str(parameter)
        default = text.partition(" = " if " = " in text else "=")[2]
        found.append((parameter.name, parameter.kind.name, default))
    return found


def normalized(raw: Iterable[tuple[str, str, str]]) -> list[tuple[str, str, str]]:
    found: list[tuple[str, str, str]] = []
    for name, kind, default in raw:
        renamed = RENAMED[name] if name in RENAMED else name
        found.append((renamed, "KEYWORD_ONLY" if renamed in KEYWORD_ONLY else kind, default))
    return found


def litellm_rows(method: str) -> list[tuple[str, str, str]]:
    done = subprocess.run(
        [sys.executable, "-c", LITELLM_PROBE, method],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return ROW.validate_json(done.stdout)


def test_async_signature_matches() -> None:
    signature = inspect.signature(CustomRoutingStrategyBase.async_get_available_deployment)
    assert normalized(parameters(signature)) == normalized(
        litellm_rows("async_get_available_deployment")
    )


def test_sync_signature_matches() -> None:
    signature = inspect.signature(CustomRoutingStrategyBase.get_available_deployment)
    theirs = normalized(litellm_rows("get_available_deployment"))
    assert normalized(parameters(signature)) == theirs


def test_keyword_only_arguments() -> None:
    for signature in (
        inspect.signature(CustomRoutingStrategyBase.async_get_available_deployment),
        inspect.signature(CustomRoutingStrategyBase.get_available_deployment),
    ):
        for name in KEYWORD_ONLY:
            assert signature.parameters[name].kind.name == "KEYWORD_ONLY"


class RecordingStrategy(CustomRoutingStrategyBase):
    @override
    def get_available_deployment(
        self, model: str, messages: list[dict[str, str]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> None: ...

    @override
    async def async_get_available_deployment(
        self, model: str, messages: list[dict[str, str]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> None: ...


def test_a_subclass_can_be_called() -> None:
    strategy = RecordingStrategy()
    assert strategy.get_available_deployment(MODEL) is None
    assert asyncio.run(strategy.async_get_available_deployment(MODEL)) is None
