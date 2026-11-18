import os
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, Unpack, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
import litellm
from litellm.llms.sagemaker.chat.transformation import SagemakerChatConfig
from pydantic import JsonValue, TypeAdapter

CALLER_KEY = "sagemaker-caller-key"


class UrlInput(TypedDict):
    function: Literal["url"]
    model: str
    region: str
    base_url: str | None
    stream: bool


class ComponentInput(TypedDict):
    function: Literal["component_header"]
    model_id: str | None
    headers: dict[str, str]


class CredentialInput(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None


class SignedInput(TypedDict):
    function: Literal["signed_request"]
    model: str
    region: str
    headers: dict[str, str]
    optional_params: dict[str, JsonValue]
    messages: list[JsonValue]
    credentials: CredentialInput
    bearer: str | None
    now: int


Input = UrlInput | ComponentInput | SignedInput
JSON = TypeAdapter(JsonValue)


class UrlArgs(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    stream: bool


class SignArgs(TypedDict):
    headers: dict[str, str]
    optional_params: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    api_base: str
    api_key: str | None
    model: str
    stream: bool
    fake_stream: bool


class ChatConfig(Protocol):
    def get_complete_url(self, **kwargs: Unpack[UrlArgs]) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> dict[str, str]: ...

    def sign_request(
        self, **kwargs: Unpack[SignArgs]
    ) -> tuple[dict[str, str], bytes | None]: ...


CONFIG: Final[ChatConfig] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L41-L44
    "ChatConfig", SagemakerChatConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L68-L70
)

litellm.suppress_debug_info = True


class FrozenClock:
    def __init__(self, frozen: datetime) -> None:
        self.frozen = frozen

    def __call__(self) -> datetime:
        return self.frozen


def run(case: Input) -> JsonValue:
    if case["function"] == "url":
        return url_value(case)
    if case["function"] == "component_header":
        optional_params: dict[str, JsonValue] = {}
        if case["model_id"] is not None:
            optional_params["model_id"] = case["model_id"]
        merged = CONFIG.validate_environment(
            case["headers"], "model", [], optional_params, {}
        )
        converted: dict[str, JsonValue] = dict(merged)
        return {"headers": converted}
    return signed_value(case)


def url_value(case: UrlInput) -> JsonValue:
    optional_params: dict[str, JsonValue] = {"aws_region_name": case["region"]}
    if case["base_url"] is not None:
        optional_params["sagemaker_base_url"] = case["base_url"]
    litellm_params: dict[str, JsonValue] = {}
    url = CONFIG.get_complete_url(
        api_base=None,
        api_key=None,
        model=case["model"],
        optional_params=optional_params,
        litellm_params=litellm_params,
        stream=case["stream"],
    )
    return {"url": url}


def signed_value(case: SignedInput) -> JsonValue:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull
    if case["bearer"] is not None:
        os.environ["AWS_BEARER_TOKEN_BEDROCK"] = case["bearer"]
    frozen = datetime.fromtimestamp(case["now"], tz=UTC).replace(tzinfo=None)
    botocore.auth.get_current_datetime = FrozenClock(frozen)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
    keys = case["credentials"]
    body_params: dict[str, JsonValue] = dict(case["optional_params"])
    signing_params: dict[str, JsonValue] = {
        **body_params,
        "aws_region_name": case["region"],
        "aws_access_key_id": keys["access_key_id"],
        "aws_secret_access_key": keys["secret_access_key"],
    }
    if keys["session_token"] is not None:
        signing_params["aws_session_token"] = keys["session_token"]
    litellm_params: dict[str, JsonValue] = {
        "custom_llm_provider": "sagemaker_chat",
        "hf_model_name": None,
    }
    request_data = CONFIG.transform_request(
        case["model"], case["messages"], body_params, litellm_params, {}
    )
    headers = CONFIG.validate_environment(
        case["headers"], case["model"], case["messages"], body_params, {}
    )
    url = CONFIG.get_complete_url(
        api_base=None,
        api_key=None,
        model=case["model"],
        optional_params=signing_params,
        litellm_params=litellm_params,
        stream=False,
    )
    signed_headers, signed_body = CONFIG.sign_request(
        headers=headers,
        optional_params=signing_params,
        request_data=request_data,
        api_base=url,
        api_key=CALLER_KEY,
        model=case["model"],
        stream=False,
        fake_stream=False,
    )
    if signed_body is None:
        raise ValueError("sign_request returned no body")
    return {
        "ok": True,
        "method": "POST",
        "url": url,
        "headers": {name.lower(): value for name, value in signed_headers.items()},
        "body": JSON.validate_json(signed_body),
    }
