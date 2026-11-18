from collections.abc import Callable
from dataclasses import dataclass
from typing import Final, NotRequired, Protocol, TypedDict, Unpack, runtime_checkable
from unittest import mock

import httpx
import litellm
import litellm.main as main
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from litellm.utils import ProviderConfigManager
from pydantic import ConfigDict, JsonValue, TypeAdapter

from anthropic_chat_call.wire import (
    SYNC_HANDLER,
    CompletionArguments,
    Recorder as LeafRecorder,
    SilentVerbose,
)

from .types import Input, Observation


class Arguments(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str
    api_key: str | None
    custom_llm_provider: str
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]
    encoding: str
    acompletion: bool
    model_response: ModelResponse
    logging_obj: "Recorder"
    timeout: int
    custom_prompt_dict: NotRequired[dict[str, JsonValue]]
    stream: NotRequired[bool]
    client: NotRequired[HTTPHandler | None]
    shared_session: NotRequired[str | None]
    print_verbose: NotRequired[Callable[[str], None]]
    logger_fn: NotRequired[None]


class PostArguments(TypedDict):
    input: list[dict[str, JsonValue]]
    api_key: str | None
    original_response: ModelResponse


@runtime_checkable
class PublicApi(Protocol):
    def completion(
        self, model: str, messages: list[dict[str, JsonValue]], custom_llm_provider: str
    ) -> ModelResponse: ...


CONFIG: Final = ConfigDict(arbitrary_types_allowed=True)
PUBLIC: Final = TypeAdapter(PublicApi, config=CONFIG).validate_python(litellm)


class Recorder:
    def __init__(self, supplied: Input) -> None:
        self.supplied = supplied
        self.result: JsonValue = None
        self.post = False
        self.post_matches = True

    def post_call(self, **kwargs: Unpack[PostArguments]) -> None:
        self.post_matches = (
            kwargs["input"] == self.supplied["messages"]
            and kwargs["original_response"].id == "chatcmpl-oracle"
        )
        self.post = True

    def completion(self, **kwargs: Unpack[Arguments]) -> ModelResponse:
        failure = self.supplied["failure"]
        if failure is not None:
            raise litellm.APIError(
                status_code=failure, message="fixture failure", model=kwargs["model"],
                llm_provider=kwargs["custom_llm_provider"],
            )
        self.result = TypeAdapter(JsonValue).validate_python({
            "model": kwargs["model"], "provider": kwargs["custom_llm_provider"],
            "model_response_id": kwargs["model_response"].id, "timeout_ms": kwargs["timeout"] * 1000,
            "api_base": kwargs["api_base"],
            "auth_matches": kwargs["api_key"] == self.supplied["expected_key"],
            "messages": kwargs["messages"], "headers": kwargs["headers"],
            "optional_params": kwargs["optional_params"], "litellm_params": kwargs["litellm_params"],
            "custom_prompt_dict": kwargs.get("custom_prompt_dict"), "stream": kwargs.get("stream"),
            "acompletion": kwargs["acompletion"],
            "client": self.supplied["client"] if kwargs.get("client") is not None else None,
            "shared_session": kwargs.get("shared_session"), "encoding": kwargs["encoding"],
            "config": "AnthropicConfig" if self.supplied["action"] == "messages" else None,
        })
        return ModelResponse(id="chatcmpl-oracle", created=1_700_000_000, model=kwargs["model"])


@dataclass
class Context:
    acompletion: bool
    api_base: str | None
    api_key: str | None
    custom_prompt_dict: dict[str, JsonValue]
    headers: dict[str, str]
    litellm_params: dict[str, JsonValue]
    logging: Recorder
    messages: list[dict[str, JsonValue]]
    model: str
    model_response: ModelResponse
    optional_params: dict[str, JsonValue]
    shared_session: str | None
    stream: bool
    timeout: int
    client: HTTPHandler | None
    custom_llm_provider: str
    logger_fn: None


def no_io(request: httpx.Request) -> httpx.Response:
    raise RuntimeError(f"unexpected external request: {request.method}")


def complete(supplied: Input) -> Observation:
    recorder = Recorder(supplied)
    with httpx.Client(transport=httpx.MockTransport(no_io)) as http:
        client = HTTPHandler(client=http)
        context = Context(
            supplied["acompletion"], supplied["api_base"], supplied["api_key"],
            supplied["custom_prompt_dict"], supplied["headers"], supplied["litellm_params"],
            recorder, supplied["messages"], supplied["model"], ModelResponse(id="chatcmpl-input"),
            supplied["optional_params"], supplied["shared_session"], supplied["stream"], 5,
            client if supplied["client"] is not None else None, supplied["provider"], None,
        )
        with (
            mock.patch.object(main, "_CompletionDispatchContext", return_value=context),
            mock.patch.object(main, "_get_encoding", return_value=supplied["encoding"]),
            mock.patch.object(main.anthropic_chat_completions, "completion", recorder.completion),
            mock.patch.object(main.base_llm_http_handler, "completion", recorder.completion),
        ):
            try:
                PUBLIC.completion(
                    model="claude-3-haiku-20240307", messages=[{"role": "user", "content": "hi"}],
                    custom_llm_provider=(
                        "anthropic_text" if supplied["action"] == "legacy" else "anthropic"
                    ),
                )
            except litellm.APIError as error:
                recorder.result = {"error": True, "status": error.status_code}
    return {
        "result": recorder.result, "post_call": recorder.post,
        "post_matches": recorder.post_matches, "request": None,
    }


def selection_failure(supplied: Input) -> Observation:
    recorder = LeafRecorder()
    with httpx.Client(transport=httpx.MockTransport(no_io)) as http:
        arguments = CompletionArguments(
            model=supplied["model"], messages=supplied["messages"],
            api_base="https://api.anthropic.com/v1/messages", api_key="oracle-key",
            custom_llm_provider=supplied["provider"], optional_params={}, litellm_params={},
            headers={}, encoding=None, acompletion=False, model_response=ModelResponse(),
            logging_obj=recorder, timeout=5, custom_prompt_dict={}, print_verbose=SilentVerbose(),
            logger_fn=None, client=HTTPHandler(client=http),
        )
        with mock.patch.object(ProviderConfigManager, "get_provider_chat_config", return_value=None):
            try:
                SYNC_HANDLER.completion(**arguments)
            except ValueError as error:
                return {
                    "result": {
                        "error": True, "status": None, "class": "ValueError", "message": str(error),
                    },
                    "post_call": False, "post_matches": True, "request": None,
                }
    raise ValueError("the handler did not reject its missing config")
