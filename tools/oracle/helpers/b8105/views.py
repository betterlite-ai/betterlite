from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.manus.files.transformation import ManusFilesConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class Sdk(TypedDict):
    api_base: str | None
    api_key: str | None


class HeaderCase(TypedDict):
    environment: dict[str, str]
    sdk: Sdk
    headers: dict[str, str]
    api_key: str | None


class UrlCase(TypedDict):
    environment: dict[str, str]
    sdk: Sdk
    api_base: str | None


class HeadersAndUrlInput(TypedDict):
    op: Literal["headers_and_url"]
    validation: list[HeaderCase]
    urls: list[UrlCase]


class FileSpec(TypedDict):
    present: bool
    filename: str | None
    content: list[int]


class PlanCase(TypedDict):
    file: FileSpec
    api_key: str | None
    api_base: str | None
    sdk: Sdk


class CreatePlanInput(TypedDict):
    op: Literal["create_plan"]
    environment: dict[str, str]
    now: int
    cases: list[PlanCase]


class ReplyCase(TypedDict):
    litellm_params: Object
    body: str


class CreateReplyInput(TypedDict):
    op: Literal["create_reply"]
    environment: dict[str, str]
    now: int
    cases: list[ReplyCase]


class RequestsInput(TypedDict):
    op: Literal["requests"]
    environment: dict[str, str]
    now: int
    sdk: Sdk
    api_base: str | None
    file_ids: list[str]
    delete_bodies: list[str]
    purposes: list[str | None]
    body: str


Input: TypeAlias = (
    HeadersAndUrlInput | CreatePlanInput | CreateReplyInput | RequestsInput
)


class CreateFileView(TypedDict):
    file: NotRequired[bytes | tuple[str | None, bytes]]
    purpose: NotRequired[str]


class FileContentView(TypedDict):
    file_id: str


class InitialUpload(TypedDict):
    method: str
    url: str
    headers: Object
    data: Object


class ContentUpload(TypedDict):
    method: str
    url: str
    headers: Object
    data: bytes


class PlanView(TypedDict):
    initial_request: InitialUpload
    upload_request: ContentUpload
    upload_url_location: str
    upload_url_key: NotRequired[str]


class FileResult(Protocol):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: str
    status: str | None
    status_details: str | None


class DeletedResult(Protocol):
    id: str
    object: str
    deleted: bool


class RequestCall(Protocol):
    def __call__(self) -> tuple[str, Object]: ...


class DeletedCall(Protocol):
    def __call__(self) -> DeletedResult: ...


class ListedCall(Protocol):
    def __call__(self) -> list[FileResult]: ...


class ConfigView(Protocol):
    def validate_environment(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L62-L93
        self,
        *,
        headers: dict[str, str],
        model: str,
        messages: list[Json],
        optional_params: Object,
        litellm_params: Object,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> Object: ...

    def get_complete_url(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L115-L138
        self,
        *,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
        stream: bool | None = None,
    ) -> str: ...

    def transform_create_file_request(
        self,
        *,
        model: str,
        create_file_data: CreateFileView,
        optional_params: Object,
        litellm_params: Object,
    ) -> PlanView: ...

    def transform_create_file_response(
        self,
        *,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: None,
        litellm_params: Object,
    ) -> FileResult: ...

    # ast-grep-ignore: no-long-names
    def transform_retrieve_file_request(
        self,
        *,
        file_id: str,
        optional_params: Object,
        litellm_params: Object,
    ) -> tuple[str, Object]: ...

    def transform_delete_file_request(
        self,
        *,
        file_id: str,
        optional_params: Object,
        litellm_params: Object,
    ) -> tuple[str, Object]: ...

    def transform_delete_file_response(
        self,
        *,
        raw_response: httpx.Response,
        logging_obj: None,
        litellm_params: Object,
    ) -> DeletedResult: ...

    def transform_list_files_request(
        self,
        *,
        purpose: str | None,
        optional_params: Object,
        litellm_params: Object,
    ) -> tuple[str, Object]: ...

    def transform_list_files_response(
        self,
        *,
        raw_response: httpx.Response,
        logging_obj: None,
        litellm_params: Object,
    ) -> list[FileResult]: ...

    def transform_file_content_request(
        self,
        *,
        file_content_request: FileContentView,
        optional_params: Object,
        litellm_params: Object,
    ) -> tuple[str, Object]: ...


def config() -> ConfigView:
    return cast("ConfigView", ManusFilesConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L55-L56
