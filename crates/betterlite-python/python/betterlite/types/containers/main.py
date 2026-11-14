from collections.abc import Callable, Mapping
from typing import ClassVar, Literal, overload

from pydantic import ConfigDict, JsonValue, PrivateAttr
from pydantic.main import IncEx
from typing_extensions import ReadOnly, TypedDict, override

from betterlite.types.utils.base import DictJsonObject, InitValue, ResponseObject


class ExpiresAfter(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    anchor: Literal["last_active_at"]
    minutes: int


class _ContainerDictObject(DictJsonObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    @override
    def __getitem__(self, key: str) -> InitValue:
        # ast-grep-ignore: no-reflection
        found: InitValue = getattr(self, key)
        return found

    @override
    def get(self, key: str, default: InitValue = None) -> InitValue:
        # ast-grep-ignore: no-reflection
        found: InitValue = getattr(self, key, default)
        return found

    @override
    def __contains__(self, key: str) -> bool:
        # ast-grep-ignore: no-reflection
        return hasattr(self, key)

    @overload
    def json(self, *args: JsonValue) -> dict[str, JsonValue]: ...

    @overload
    def json(
        self, *, include: IncEx | None = None, exclude: IncEx | None = None,
        by_alias: bool = False, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
        encoder: Callable[[JsonValue], JsonValue] | None = None,
        models_as_dict: bool = False, **dumps_kwargs: JsonValue,
    ) -> str: ...

    @override
    def json(
        self, *_args: JsonValue, mode: str = "python", include: IncEx | None = None,
        exclude: IncEx | None = None, context: JsonValue = None,
        by_alias: bool | None = None, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
        exclude_computed_fields: bool = False, round_trip: bool = False,
        warnings: bool | Literal["none", "warn", "error"] = True,
        fallback: Callable[[JsonValue], JsonValue] | None = None,
        serialize_as_any: bool = False, polymorphic_serialization: bool | None = None,
        encoder: Callable[[JsonValue], JsonValue] | None = None,
        models_as_dict: bool = False, **dumps_kwargs: JsonValue,
    ) -> JsonValue:
        if _args:
            raise TypeError("json() takes no positional arguments")
        if encoder is not None or models_as_dict or dumps_kwargs:
            raise TypeError("json() takes only the model_dump options")
        dumped: dict[str, JsonValue] = self.model_dump(
            mode=mode, include=include, exclude=exclude, context=context,
            by_alias=by_alias, exclude_unset=exclude_unset,
            exclude_defaults=exclude_defaults, exclude_none=exclude_none,
            exclude_computed_fields=exclude_computed_fields, round_trip=round_trip,
            warnings=warnings, fallback=fallback, serialize_as_any=serialize_as_any,
            polymorphic_serialization=polymorphic_serialization,
        )
        return dumped


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
