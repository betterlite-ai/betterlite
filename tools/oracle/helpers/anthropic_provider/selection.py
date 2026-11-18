from typing import Final, Protocol, runtime_checkable

from litellm.llms.base_llm.chat.transformation import BaseConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import LlmProviders, ModelResponse
from litellm.utils import ProviderConfigManager
from pydantic import ConfigDict, TypeAdapter
from typing_extensions import override

from anthropic_chat_call.wire import SYNC_HANDLER, CompletionArguments, Recorder, SilentVerbose

from .types import Input, Observation


class Lookup(Protocol):
    def __call__(self, *, model: str, provider: LlmProviders) -> BaseConfig | None: ...


class Missing(Lookup):
    @override
    def __call__(self, *, model: str, provider: LlmProviders) -> None:
        return None


@runtime_checkable
class Manager(Protocol):
    get_provider_chat_config: Lookup


MANAGER: Final = TypeAdapter(
    Manager, config=ConfigDict(arbitrary_types_allowed=True),
).validate_python(ProviderConfigManager)


def selection_failure(supplied: Input, client: HTTPHandler) -> Observation:
    arguments = CompletionArguments(
        model=supplied["model"], messages=supplied["messages"],
        api_base="https://api.anthropic.com/v1/messages", api_key="oracle-key",
        custom_llm_provider=supplied["provider"], optional_params={}, litellm_params={},
        headers={}, encoding=None, acompletion=False, model_response=ModelResponse(),
        logging_obj=Recorder(), timeout=5, custom_prompt_dict={}, print_verbose=SilentVerbose(),
        logger_fn=None, client=client,
    )
    original = MANAGER.get_provider_chat_config
    MANAGER.get_provider_chat_config = Missing()
    try:
        SYNC_HANDLER.completion(**arguments)
    except ValueError as error:
        return {
            "result": {
                "error": True, "status": None, "class": "ValueError", "message": str(error),
            },
            "post_call": False, "post_matches": True, "request": None,
        }
    finally:
        MANAGER.get_provider_chat_config = original
    raise ValueError("the handler did not reject its missing config")
