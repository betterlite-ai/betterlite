import os
from datetime import UTC, datetime
from pathlib import Path
from typing import Final, Literal, NotRequired, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.chatgpt.authenticator import Authenticator
from litellm.llms.chatgpt.responses.transformation import ChatGPTResponsesAPIConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

# get_llm_provider prints the provider list to stdout unless this is set
litellm.suppress_debug_info = True

LOG_START: Final = datetime(2026, 1, 1, tzinfo=UTC)
REQUEST_URL: Final = "https://chatgpt.com/backend-api/codex/responses"


class Input(TypedDict):
    step: Literal["request", "response"]
    environment: dict[str, str]
    model: str
    input: str
    options: dict[str, JsonValue]
    body: str
    status: int
    headers: dict[str, str]
    now: int
    api_base: str
    access_token: str
    account_id: str | None


class ItemView(TypedDict):
    id: str
    type: NotRequired[str]


class ReplyView(TypedDict):
    id: str
    created_at: int
    model: str | None
    object: str | None
    status: str | None
    output: list[ItemView]


class PostCallView(BaseModel):
    original_response: str
    additional_args: dict[str, JsonValue]
    log_event_type: str


class Recorded(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: PostCallView


class HiddenView(BaseModel):
    additional_headers: dict[str, JsonValue]


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    params: HiddenView = Field(alias="_hidden_params")


@runtime_checkable
class ResponsesConfig(Protocol):
    def transform_responses_api_request(
        self,
        model: str,
        input_text: str,
        response_api_optional_request_params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def transform_response_api_response(
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
        /,
    ) -> ResponsesAPIResponse: ...


class Bound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: ResponsesConfig


class ScriptedAuthenticator(Authenticator):
    def __init__(self, api_base: str, access_token: str, account_id: str | None) -> None:
        self.base_value = api_base
        self.token_value = access_token
        self.account_value = account_id

    @override
    def get_api_base(self) -> str:
        return self.base_value

    @override
    def get_access_token(self) -> str:
        return self.token_value

    @override
    def get_account_id(self) -> str | None:
        return self.account_value


REPLY: Final[TypeAdapter[ReplyView]] = TypeAdapter(ReplyView)


def logging_object() -> Logging:
    return Logging(
        model="gpt-5",
        messages=None,
        stream=True,
        call_type="responses",
        start_time=LOG_START,
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def response_value(config: ResponsesConfig, input_value: Input) -> dict[str, JsonValue]:
    logging_obj = logging_object()
    response = httpx.Response(
        input_value["status"],
        stream=httpx.ByteStream(input_value["body"].encode()),
        headers=input_value["headers"],
        request=httpx.Request("POST", REQUEST_URL),
    )
    response.read()
    reply = config.transform_response_api_response(input_value["model"], response, logging_obj)
    view = REPLY.validate_json(reply.model_dump_json())
    hidden = Hidden.model_validate(reply, from_attributes=True)
    post = Recorded.model_validate(logging_obj, from_attributes=True).model_call_details
    outputs: list[JsonValue] = [
        {"type": item.get("type"), "id": item["id"]} for item in view["output"]
    ]
    value: dict[str, JsonValue] = {
        "id": view["id"],
        "created_at": view["created_at"],
        "model": view["model"],
        "object": view["object"],
        "status": view["status"],
        "output_text": reply.output_text,
        "output": outputs,
    }
    post_call: dict[str, JsonValue] = {
        "original_response": post.original_response,
        "additional_args": post.additional_args,
        "log_event_type": post.log_event_type,
    }
    return {
        "value": value,
        "headers": {name: header_value for name, header_value in input_value["headers"].items()},
        "additional_headers": hidden.params.additional_headers,
        "post_call": post_call,
    }


def run(input_value: Input) -> JsonValue:
    os.environ["CHATGPT_TOKEN_DIR"] = str(Path(__file__).resolve().parent)
    concrete = ChatGPTResponsesAPIConfig()
    concrete.authenticator = ScriptedAuthenticator(
        input_value["api_base"], input_value["access_token"], input_value["account_id"]
    )
    holder: dict[str, ChatGPTResponsesAPIConfig] = {"config": concrete}
    config = Bound.model_validate(holder).config
    if input_value["step"] == "request":
        return config.transform_responses_api_request(
            input_value["model"],
            input_value["input"],
            input_value["options"],
            GenericLiteLLMParams(),
            {},
        )
    return response_value(config, input_value)
