import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from functools import partial
from typing import Final, Protocol, Unpack, runtime_checkable

import litellm
from litellm import main
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

from .types import Arguments, Context, Input


class NativeCompletion(Protocol):
    def completion(self, **kwargs: Unpack[Arguments]) -> ModelResponse: ...


ContextValue = JsonValue | ModelResponse | Logging | HTTPHandler


class ContextFactory(Protocol):
    def __call__(self, **kwargs: ContextValue) -> Context: ...


class FixedContext(ContextFactory):
    def __init__(self, context: Context) -> None:
        self.context = context

    @override
    def __call__(self, **_kwargs: ContextValue) -> Context:
        return self.context


@runtime_checkable
class MainFields(Protocol):
    _CompletionDispatchContext: ContextFactory
    _get_encoding: Callable[[], str]
    anthropic_chat_completions: NativeCompletion
    base_llm_http_handler: NativeCompletion


CONFIG: Final = ConfigDict(arbitrary_types_allowed=True)
MAIN: Final = TypeAdapter(MainFields, config=CONFIG).validate_python(main)


class NativeBindings(MainFields):
    # The public completion entry constructs this context before choosing its native branch.
    @contextmanager
    def install(
        self: MainFields, context: Context, recorder: NativeCompletion, encoding: str,
    ) -> Generator[None]:
        context_factory = self._CompletionDispatchContext
        get_encoding = self._get_encoding
        chat = self.anthropic_chat_completions
        legacy = self.base_llm_http_handler
        self._CompletionDispatchContext: ContextFactory = FixedContext(context)
        self._get_encoding: Callable[[], str] = partial(str, encoding)
        self.anthropic_chat_completions = recorder
        self.base_llm_http_handler = recorder
        try:
            yield
        finally:
            self._CompletionDispatchContext = context_factory
            self._get_encoding = get_encoding
            self.anthropic_chat_completions = chat
            self.base_llm_http_handler = legacy


@runtime_checkable
class Globals(Protocol):
    api_base: str | None
    api_key: str | None
    anthropic_key: str | None
    custom_prompt_dict: dict[str, JsonValue]


GLOBALS: Final = TypeAdapter(Globals, config=CONFIG).validate_python(litellm)


@contextmanager
def environment(supplied: Input) -> Generator[None]:
    original_env = dict(os.environ)
    original = (
        GLOBALS.api_base, GLOBALS.api_key, GLOBALS.anthropic_key, GLOBALS.custom_prompt_dict,
    )
    os.environ.clear()
    os.environ.update(supplied["environment"])
    os.environ["LITELLM_RUST"] = "false"
    os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "true"
    os.environ["LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX"] = str(supplied["disable_suffix"]).lower()
    GLOBALS.api_base = supplied["global_base"]
    GLOBALS.api_key = supplied["global_key"]
    GLOBALS.anthropic_key = supplied["anthropic_key"]
    GLOBALS.custom_prompt_dict = supplied["global_prompt_dict"]
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(original_env)
        (
            GLOBALS.api_base, GLOBALS.api_key, GLOBALS.anthropic_key, GLOBALS.custom_prompt_dict,
        ) = original
