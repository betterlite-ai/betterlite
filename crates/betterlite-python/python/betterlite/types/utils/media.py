import json
import time
from collections.abc import Mapping, Sequence
from typing import Literal, TypeAlias, Unpack

from pydantic import JsonValue, PrivateAttr
from pydantic.main import IncEx
from typing_extensions import override

from betterlite.types.utils.base import (
    DumpOptions,
    InitValue,
    JsonDumpOptions,
    ModelValue,
    ResponseObject,
    dump_value,
    validated_value,
)

Selection: TypeAlias = IncEx


class ImageObject(ResponseObject):
    b64_json: str | None = None
    url: str | None = None
    revised_prompt: str | None = None
    provider_specific_fields: dict[str, JsonValue] | None = None

    @override
    def __init__(
        self, b64_json: JsonValue = None, url: JsonValue = None,
        revised_prompt: JsonValue = None,
        provider_specific_fields: Mapping[str, JsonValue] | None = None,
        **_params: InitValue,
    ) -> None:
        super().__init__(b64_json=b64_json, url=url, revised_prompt=revised_prompt)
        if provider_specific_fields:
            self.provider_specific_fields = dict(provider_specific_fields)


class ImageUsageInputTokensDetails(ResponseObject):
    image_tokens: int
    text_tokens: int


class ImageUsage(ResponseObject):
    input_tokens: int
    input_tokens_details: ImageUsageInputTokensDetails
    output_tokens: int
    total_tokens: int


class ImageResponse(ResponseObject):
    created: int
    background: Literal["transparent", "opaque"] | None = None
    data: list[ImageObject]
    output_format: Literal["png", "webp", "jpeg"] | None = None
    quality: Literal["low", "medium", "high"] | None = None
    size: Literal["1024x1024", "1024x1536", "1536x1024"] | None = None
    usage: ImageUsage | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(
        self, created: JsonValue = None, data: Sequence[ModelValue] | None = None,
        response_ms: float | None = None, usage: ModelValue = None,
        hidden_params: Mapping[str, JsonValue] | None = None,
        background: JsonValue = None, quality: JsonValue = None,
        output_format: JsonValue = None, size: JsonValue = None,
        **_params: InitValue,
    ) -> None:
        images: list[ImageObject] = (
            [] if data is None
            else [validated_value(image, ImageObject) for image in data]
        )
        if usage is None:
            selected_usage: InitValue = ImageUsage(
                input_tokens=0,
                input_tokens_details=ImageUsageInputTokensDetails(image_tokens=0, text_tokens=0),
                output_tokens=0,
                total_tokens=0,
            )
        else:
            selected_usage = dump_value(usage)
        super().__init__(
            created=int(time.time()) if created is None else created,
            background=background, data=images, output_format=output_format,
            quality=quality, size=size, usage=selected_usage,
        )
        if hidden_params is not None:
            self._hidden_params = dict(hidden_params)

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        include = kwargs.get("include")
        exclude = kwargs.get("exclude")
        exclude_unset = kwargs.get("exclude_unset")
        exclude_none = kwargs.get("exclude_none")
        mode = kwargs.get("mode")
        dumped: dict[str, JsonValue] = super().model_dump(
            mode="python" if mode is None else mode,
            include=top_fields(include, is_include=True),
            exclude=top_fields(exclude, is_include=False),
            exclude_unset=exclude_unset is True, exclude_none=exclude_none is True,
        )
        if "data" in dumped:
            images: list[JsonValue] = []
            count = len(self.data)
            selected_include = data_selector(include)
            selected_exclude = data_selector(exclude)
            for index, image in enumerate(self.data):
                include_keep, include_selector = nested_selector(
                    selector=selected_include, index=index, count=count, is_include=True,
                )
                exclude_keep, exclude_selector = nested_selector(
                    selector=selected_exclude, index=index, count=count, is_include=False,
                )
                if not include_keep or not exclude_keep:
                    continue
                images.append(image.model_dump(
                    mode="python" if mode is None else mode, include=include_selector,
                    exclude=exclude_selector, exclude_unset=exclude_unset is True,
                    exclude_none=exclude_none is True,
                ))
            dumped["data"] = images
        return dumped

    @override
    def model_dump_json(self, **kwargs: Unpack[JsonDumpOptions]) -> str:
        dumped = self.model_dump(
            include=kwargs.get("include"), exclude=kwargs.get("exclude"),
            exclude_unset=kwargs.get("exclude_unset") is True,
            exclude_none=kwargs.get("exclude_none") is True,
        )
        indent = kwargs.get("indent")
        ensure_ascii = kwargs.get("ensure_ascii")
        return json.dumps(
            dumped, indent=indent, ensure_ascii=ensure_ascii is True,
        )


def top_fields(selector: IncEx | None, *, is_include: bool) -> set[str] | None:
    if selector is None:
        return None
    match selector:
        case Mapping():
            return {str(key) for key, value in selector.items() if is_include or value is True}
        case set():
            return {str(item) for item in selector}


def data_selector(selector: IncEx | None) -> IncEx | bool | None:
    if selector is None:
        return None
    match selector:
        case Mapping():
            found, value = mapping_value(selector, "data")
            if not found:
                return None
            return value
        case set():
            return None


def nested_selector(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L94-L111
    *, selector: IncEx | bool | None, index: int, count: int, is_include: bool,
) -> tuple[bool, IncEx | None]:
    if selector is None or selector is True:
        return True, None
    if selector is False:
        return False, None
    match selector:
        case Mapping():
            found, value = mapping_value(selector, index)
            if not found:
                found, value = mapping_value(selector, index - count)
            if not found:
                found, value = mapping_value(selector, "__all__")
            keep = value is not None if is_include else value is not True
            per_item = None if value is True or value is None or value is False else value
            return keep, per_item
        case set():
            indexes = int_indexes(selector)
            if indexes is None:
                return True, selector
            addressed = index in indexes or index - count in indexes
            return (addressed if is_include else not addressed), None


def mapping_value(
    selector: Mapping[int, IncEx | bool] | Mapping[str, IncEx | bool], key: int | str,
) -> tuple[bool, IncEx | bool | None]:
    for existing, value in selector.items():
        match existing:
            case int():
                if existing == key:
                    return True, value
            case str():
                if existing == key:
                    return True, value
    return False, None


def int_indexes(selector: set[int] | set[str]) -> list[int] | None:
    indexes: list[int] = []
    for item in selector:
        match item:
            case int():
                indexes.append(item)
            case str():
                return None
    return indexes
