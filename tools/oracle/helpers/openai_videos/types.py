from typing import Annotated, Literal, TypeAlias, TypedDict

from pydantic import Field, JsonValue, TypeAdapter


class TextReference(TypedDict):
    kind: Literal["text"]
    text: str


class BytesReference(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


Reference: TypeAlias = Annotated[TextReference | BytesReference, Field(discriminator="kind")]


class CreateCall(TypedDict):
    op: Literal["create"]
    model: str
    prompt: str
    api_base: str
    options: dict[str, JsonValue]
    reference: Reference | None


class CreateReplyCall(TypedDict):
    op: Literal["create_response"]
    model: str
    provider: str | None
    body_base64: str


class ContentRequestCall(TypedDict):
    op: Literal["content_request"]
    video_id: str
    api_base: str
    variant: str | None


class ContentReplyCall(TypedDict):
    op: Literal["content_response"]
    body_base64: str


class RemixRequestCall(TypedDict):
    op: Literal["remix_request"]
    video_id: str
    prompt: str
    api_base: str


class RemixReplyCall(TypedDict):
    op: Literal["remix_response"]
    provider: str | None
    body_base64: str


class ListRequestCall(TypedDict):
    op: Literal["list_request"]
    api_base: str
    after: str | None
    limit: int | None
    order: str | None


class ListReplyCall(TypedDict):
    op: Literal["list_response"]
    provider: str | None
    body_base64: str


class DeleteRequestCall(TypedDict):
    op: Literal["delete_request"]
    video_id: str
    api_base: str


class DeleteReplyCall(TypedDict):
    op: Literal["delete_response"]
    body_base64: str


class StatusRequestCall(TypedDict):
    op: Literal["status_request"]
    video_id: str
    api_base: str


class StatusReplyCall(TypedDict):
    op: Literal["status_response"]
    provider: str | None
    body_base64: str


class EnvironmentCall(TypedDict):
    op: Literal["validate_environment"]
    headers: dict[str, str]
    api_key: str | None
    params_api_key: str | None
    global_api_key: str | None
    openai_key: str | None
    environment_api_key: str | None


class CompleteCall(TypedDict):
    op: Literal["complete_url"]
    api_base: str | None


class ErrorCall(TypedDict):
    op: Literal["error_class"]
    message: str
    status: int
    headers: dict[str, str]


Call: TypeAlias = Annotated[
    (
        CreateCall | CreateReplyCall | ContentRequestCall | ContentReplyCall
        | RemixRequestCall | RemixReplyCall | ListRequestCall | ListReplyCall
        | DeleteRequestCall | DeleteReplyCall | StatusRequestCall | StatusReplyCall
        | EnvironmentCall | CompleteCall | ErrorCall
    ),
    Field(discriminator="op"),
]
CALLS: TypeAdapter[list[Call]] = TypeAdapter(list[Call])

VideoCreateOps: TypeAlias = CreateCall | CreateReplyCall
VideoRequestOps: TypeAlias = (
    ContentRequestCall | RemixRequestCall | ListRequestCall | DeleteRequestCall | StatusRequestCall
)
VideoReplyOps: TypeAlias = (
    ContentReplyCall | RemixReplyCall | ListReplyCall | DeleteReplyCall | StatusReplyCall
)
VideoEnvOps: TypeAlias = EnvironmentCall | CompleteCall | ErrorCall


class VideoView(TypedDict):
    id: str
    object: Literal["video"]
    status: str
    created_at: int | None
    completed_at: int | None
    expires_at: int | None
    error: dict[str, JsonValue] | None
    progress: int | None
    remixed_from_video_id: str | None
    seconds: str | None
    size: str | None
    model: str | None
    usage: dict[str, JsonValue] | None


class VideoListView(TypedDict):
    data: list[VideoView]
    first_id: str | None
    last_id: str | None
    object: str | None
    has_more: bool | None


VIDEO_VIEW: TypeAdapter[VideoView] = TypeAdapter(VideoView)
VIDEO_LIST: TypeAdapter[VideoListView] = TypeAdapter(VideoListView)
