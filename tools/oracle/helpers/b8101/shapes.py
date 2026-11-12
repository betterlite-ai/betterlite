from typing import Literal, TypeAlias, TypedDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

MODEL = "langflow/abc"


class ProviderCase(TypedDict):
    api_base: str | None
    api_key: str | None


class UrlCase(TypedDict):
    model: str
    options: Object
    api_base: str | None


class ValidateCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None


class RequestCase(TypedDict):
    messages: list[Object]
    options: Object


class SignCase(TypedDict):
    headers: dict[str, str]
    request_data: Object


class ReplyCase(TypedDict):
    model: str
    status: int
    body: str
    messages: list[Object]


class FlowInput(TypedDict):
    op: Literal["flow"]
    provider: ProviderCase
    urls: list[UrlCase]
    validate: list[ValidateCase]


class RequestsInput(TypedDict):
    op: Literal["request"]
    requests: list[RequestCase]
    signatures: list[SignCase]


class RepliesInput(TypedDict):
    op: Literal["reply"]
    replies: list[ReplyCase]


Input: TypeAlias = FlowInput | RequestsInput | RepliesInput
