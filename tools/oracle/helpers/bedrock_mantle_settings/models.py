from typing import Final, Literal, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str]


class BaseCase(TypedDict):
    model: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    aws_region_name: NotRequired[str]
    inject_signer: NotRequired[bool]
    environment: NotRequired[dict[str, str]]


class BasePayload(TypedDict):
    model_cost: NotRequired[dict[str, JsonValue]]
    cases: list[BaseCase]


class SignCase(TypedDict):
    api_base: str
    headers: NotRequired[list[tuple[str, str | None]]]
    optional_params: NotRequired[dict[str, JsonValue]]
    request_data: NotRequired[dict[str, JsonValue]]
    api_key: NotRequired[str]
    model: NotRequired[str]
    stream: NotRequired[bool]
    fake_stream: NotRequired[bool]
    credentials: Keys
    environment: NotRequired[dict[str, str]]
    now: int


class SignPayload(TypedDict):
    cases: list[SignCase]


class Catalog(TypedDict):
    chat_models: NotRequired[list[str]]
    text_models: NotRequired[list[str]]


class ErrorProbe(TypedDict):
    message: str
    status_code: int
    headers: NotRequired[dict[str, str]]


class ChatCase(TypedDict):
    model: str
    project_id: NotRequired[str]
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    reasoning: NotRequired[Literal["supported", "unsupported", "panic"]]
    error: ErrorProbe
    json_mode: NotRequired[bool | None]
    sync_stream: NotRequired[bool]
    environment: NotRequired[dict[str, str]]


class ChatPayload(TypedDict):
    catalog: NotRequired[Catalog]
    cases: list[ChatCase]


class MessageCase(TypedDict):
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class MessagePayload(TypedDict):
    cases: list[MessageCase]


class PassthroughCase(TypedDict):
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    aws_bedrock_runtime_endpoint: NotRequired[str]
    endpoint_type: NotRequired[Literal["runtime", "agent", "agentcore"]]
    api_key: NotRequired[JsonValue]
    environment: NotRequired[dict[str, str]]


class PassthroughPayload(TypedDict):
    cases: list[PassthroughCase]


FailureKind = Literal[
    "no_credentials",
    "partial_credentials",
    "profile_not_found",
    "credential_retrieval",
    "region",
]


class FailureCase(TypedDict):
    kind: FailureKind
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class FailurePayload(TypedDict):
    cases: list[FailureCase]


class ReplyCase(TypedDict):
    custom_llm_provider: str
    endpoint: str


class ReplyPayload(TypedDict):
    cases: list[ReplyCase]


BASE_PAYLOAD: Final = TypeAdapter(BasePayload)
SIGN_PAYLOAD: Final = TypeAdapter(SignPayload)
CHAT_PAYLOAD: Final = TypeAdapter(ChatPayload)
MESSAGE_PAYLOAD: Final = TypeAdapter(MessagePayload)
PASSTHROUGH_PAYLOAD: Final = TypeAdapter(PassthroughPayload)
FAILURE_PAYLOAD: Final = TypeAdapter(FailurePayload)
REPLY_PAYLOAD: Final = TypeAdapter(ReplyPayload)
