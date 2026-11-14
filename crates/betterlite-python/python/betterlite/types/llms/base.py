from collections.abc import Callable, ItemsView
from typing import ClassVar, Literal, Never, Unpack, overload

from pydantic import BaseModel, ConfigDict, JsonValue, PrivateAttr
from pydantic.main import IncEx
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    DumpOptions,
    InitValue,
)


class LiteLLMPydanticObjectBase(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(protected_namespaces=())

    @overload
    def json(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]: ...

    @overload
    def json(
        self, *, include: IncEx | None = None, exclude: IncEx | None = None,
        by_alias: bool = False, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
        encoder: Callable[[JsonValue], JsonValue] | None = None,
        models_as_dict: bool = False, **dumps_kwargs: Never,
    ) -> str: ...

    @override
    def json(
        self, *, mode: str = "python", include: IncEx | None = None,
        exclude: IncEx | None = None, context: JsonValue = None,
        by_alias: bool | None = None, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
        exclude_computed_fields: bool = False, round_trip: bool = False,
        warnings: bool | Literal["none", "warn", "error"] = True,
        fallback: Callable[[JsonValue], JsonValue] | None = None,
        serialize_as_any: bool = False, polymorphic_serialization: bool | None = None,
        encoder: Callable[[JsonValue], JsonValue] | None = None,
        models_as_dict: bool = False, **dumps_kwargs: Never,
    ) -> JsonValue:
        # the second overload only matches pydantic's deprecated signature; LiteLLM dumps here
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

    def fields_set(self) -> set[str]:
        return self.model_fields_set


# ast-grep-ignore: no-long-names
class BaseLiteLLMOpenAIResponseObject(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    def __getitem__(self, key: str) -> InitValue:
        if key not in type(self).model_fields:
            raise KeyError(key)
        value: InitValue = self.__getattribute__(key)
        return value

    def get(self, key: str, default: InitValue = None) -> InitValue:
        if key not in type(self).model_fields:
            return default
        return self[key]

    def __contains__(self, key: str) -> bool:
        return key in type(self).model_fields

    def items(self) -> ItemsView[str, InitValue]:
        return {name: self[name] for name in type(self).model_fields}.items()


class HiddenParams(DictJsonObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    original_response: JsonValue = None
    model_id: str | None = None
    api_base: str | None = None
    _response_ms: float | None = PrivateAttr(default=None)
    response_cost: float | None = None

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        data: dict[str, JsonValue] = super().model_dump(**kwargs)
        data["_response_ms"] = self._response_ms
        return data


class CachedTokensDetails(BaseModel):
    text_tokens: int | None = None
    audio_tokens: int | None = None
    image_tokens: int | None = None
