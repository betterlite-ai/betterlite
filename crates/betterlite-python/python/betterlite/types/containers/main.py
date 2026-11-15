from collections.abc import Mapping
from typing import ClassVar, Literal

from pydantic import ConfigDict, JsonValue, PrivateAttr
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.llms.base import LiteLLMPydanticObjectBase
from betterlite.types.utils.base import InitValue, ResponseObject


class ExpiresAfter(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    anchor: Literal["last_active_at"]
    minutes: int


class _ContainerDictObject(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    def __getitem__(self, key: str) -> InitValue:
        # ast-grep-ignore: no-reflection
        found: InitValue = getattr(self, key)
        return found

    def get(self, key: str, default: InitValue = None) -> InitValue:
        # ast-grep-ignore: no-reflection
        found: InitValue = getattr(self, key, default)
        return found

    def __contains__(self, key: str) -> bool:
        # ast-grep-ignore: no-reflection
        return hasattr(self, key)


class ContainerObject(_ContainerDictObject):
    id: str
    object: Literal["container"]
    created_at: int
    status: str
    expires_after: ExpiresAfter | None = None
    last_active_at: int | None = None
    name: str | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


class DeleteContainerResult(_ContainerDictObject):
    id: str
    object: Literal["container.deleted"]
    deleted: bool


class ContainerListResponse(_ContainerDictObject):
    object: Literal["list"]
    data: list[ContainerObject]
    first_id: str | None = None
    last_id: str | None = None
    has_more: bool


# ast-grep-ignore: no-long-names
class ContainerCreateOptionalRequestParams(TypedDict, total=False):
    expires_after: ReadOnly[Mapping[str, JsonValue] | None]
    file_ids: list[str] | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None


# ast-grep-ignore: no-long-names
class ContainerCreateRequestParams(ContainerCreateOptionalRequestParams, total=False):
    name: str


# ast-grep-ignore: no-long-names
class ContainerListOptionalRequestParams(TypedDict, total=False):
    after: str | None
    limit: int | None
    order: str | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, str] | None


class ContainerFileObject(_ContainerDictObject):
    id: str
    object: Literal["container.file", "container_file"]
    container_id: str
    bytes: int | None = None
    created_at: int
    path: str
    source: str
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


class ContainerFileListResponse(_ContainerDictObject):
    object: Literal["list"]
    data: list[ContainerFileObject]
    first_id: str | None = None
    last_id: str | None = None
    has_more: bool


class DeleteContainerFileResponse(_ContainerDictObject):
    id: str
    object: Literal["container.file.deleted", "container_file.deleted"]
    deleted: bool
