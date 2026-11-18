import asyncio
import inspect
from typing import Final

from litellm.types.router import CustomRoutingStrategyBase as LiteLLMStrategy
from pydantic import JsonValue
from typing_extensions import override

from betterlite._custom_routing import CustomRoutingStrategyBase

MODEL: Final = "gpt-4o"
RENAMED: Final = {"input": "input_value"}
KEYWORD_ONLY: Final = frozenset({"specific_deployment", "request_kwargs"})


def rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    found: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        name = parameter.name
        if name in RENAMED:
            name = RENAMED[name]
        kind = "KEYWORD_ONLY" if name in KEYWORD_ONLY else parameter.kind.name
        # str(Parameter) spells the default after " = ", or after "=" when it has no annotation
        text = str(parameter)
        default = text.partition(" = " if " = " in text else "=")[2]
        found.append((name, kind, default))
    return found


def test_async_signature_matches() -> None:
    ours = rows(inspect.signature(CustomRoutingStrategyBase.async_get_available_deployment))
    theirs = rows(inspect.signature(LiteLLMStrategy.async_get_available_deployment))
    assert ours == theirs


def test_sync_signature_matches() -> None:
    ours = rows(inspect.signature(CustomRoutingStrategyBase.get_available_deployment))
    theirs = rows(inspect.signature(LiteLLMStrategy.get_available_deployment))
    assert ours == theirs


def test_keyword_only_arguments() -> None:
    for signature in (
        inspect.signature(CustomRoutingStrategyBase.async_get_available_deployment),
        inspect.signature(CustomRoutingStrategyBase.get_available_deployment),
    ):
        for name in KEYWORD_ONLY:
            assert signature.parameters[name].kind is inspect.Parameter.KEYWORD_ONLY


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
