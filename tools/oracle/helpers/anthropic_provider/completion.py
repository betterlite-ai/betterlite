from typing import Final, Protocol, Unpack, runtime_checkable

import httpx
import litellm
from litellm.exceptions import APIError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

from .bindings import MAIN, NativeBindings
from .selection import selection_failure
from .types import Arguments, Context, Input, Observation, PostArguments


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
            raise APIError(
                status_code=failure, message="fixture failure", model=kwargs["model"],
                llm_provider=kwargs["custom_llm_provider"],
            )
        captured: dict[str, JsonValue] = {
            "model": kwargs["model"], "provider": kwargs["custom_llm_provider"],
            "model_response_id": kwargs["model_response"].id,
            "timeout_ms": kwargs["timeout"] * 1000,
            "api_base": kwargs["api_base"],
            "auth_matches": kwargs["api_key"] == self.supplied["expected_key"],
            "messages": TypeAdapter(JsonValue).validate_python(kwargs["messages"]),
            "headers": TypeAdapter(JsonValue).validate_python(kwargs["headers"]),
            "optional_params": kwargs["optional_params"],
            "litellm_params": kwargs["litellm_params"],
            "custom_prompt_dict": kwargs.get("custom_prompt_dict"), "stream": kwargs.get("stream"),
            "acompletion": kwargs["acompletion"],
            "client": self.supplied["client"] if kwargs.get("client") is not None else None,
            "shared_session": kwargs.get("shared_session"), "encoding": kwargs["encoding"],
            "config": "AnthropicConfig" if self.supplied["action"] == "messages" else None,
        }
        self.result = captured
        return ModelResponse(id="chatcmpl-oracle", created=1_700_000_000, model=kwargs["model"])


class NoNetwork(httpx.BaseTransport):
    @override
    def handle_request(self, request: httpx.Request) -> httpx.Response:
        raise RuntimeError(f"unexpected external request: {request.method}")


def complete(supplied: Input) -> Observation:
    recorder = Recorder(supplied)
    with httpx.Client(transport=NoNetwork()) as http:
        client = HTTPHandler(client=http)
        if not supplied["config_present"] or supplied["provider"] == "not-a-provider":
            return selection_failure(supplied, client)
        context = Context(
            supplied["acompletion"], supplied["api_base"], supplied["api_key"],
            supplied["custom_prompt_dict"], supplied["headers"], supplied["litellm_params"],
            recorder, supplied["messages"], supplied["model"], ModelResponse(id="chatcmpl-input"),
            supplied["optional_params"], supplied["shared_session"], supplied["stream"], 5,
            client if supplied["client"] is not None else None, supplied["provider"], None,
        )
        with NativeBindings.install(MAIN, context, recorder, supplied["encoding"]):
            try:
                PUBLIC.completion(
                    model="claude-3-haiku-20240307", messages=[{"role": "user", "content": "hi"}],
                    custom_llm_provider=(
                        "anthropic_text" if supplied["action"] == "legacy" else "anthropic"
                    ),
                )
            except APIError as error:
                recorder.result = {"error": True, "status": error.status_code}
    return {
        "result": recorder.result, "post_call": recorder.post,
        "post_matches": recorder.post_matches, "request": None,
    }
