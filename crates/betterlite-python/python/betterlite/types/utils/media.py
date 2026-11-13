import time
from collections.abc import Collection, Mapping
from typing import ClassVar, Literal, TypeAlias, TypeGuard

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import JsonValue, ModelObject, StoredValue

SelectorItems: TypeAlias = (
    list[int] | list[str] | set[int] | set[str] | frozenset[int] | frozenset[str]
    | tuple[int, ...] | tuple[str, ...]
)
NestedSelector: TypeAlias = bool | SelectorItems | Mapping[str, bool] | Mapping[int, bool]
FieldSelector: TypeAlias = (
    NestedSelector | Mapping[int, NestedSelector] | Mapping[str, NestedSelector]
)
Selection: TypeAlias = Collection[str] | Mapping[str, FieldSelector]


class ImageObject(ResponseObject):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None

    @override
    def __init__(
        self, b64_json: StoredValue = None, url: StoredValue = None,
        revised_prompt: StoredValue = None, provider_specific_fields: StoredValue = None,
        **_params: StoredValue,
    ) -> None:
        fields: dict[str, StoredValue] = {
            "b64_json": b64_json, "url": url, "revised_prompt": revised_prompt,
        }
        if provider_specific_fields is not None:
            fields["provider_specific_fields"] = provider_specific_fields
        super().__init__(**fields)


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
    background: Literal["transparent", "opaque"] | None
    data: list[ImageObject]
    output_format: Literal["png", "webp", "jpeg"] | None
    quality: Literal["low", "medium", "high"] | None
    size: Literal["1024x1024", "1024x1536", "1536x1024"] | None
    usage: ImageUsage | None
    _hidden_params: dict[str, StoredValue]
    defaults: ClassVar[dict[str, StoredValue]] = {"_hidden_params": {}}

    @override
    def __init__(
        self, created: StoredValue = None, data: StoredValue = None,
        response_ms: float | None = None, usage: StoredValue = None,
        hidden_params: Mapping[str, StoredValue] | None = None,
        background: StoredValue = None, quality: StoredValue = None,
        output_format: StoredValue = None, size: StoredValue = None,
        **_params: StoredValue,
    ) -> None:
        images: list[StoredValue] = []
        match data:
            case list():
                images = [image_object(image) for image in data]
            case _other:
                images = []
        super().__init__(
            created=int(time.time()) if created is None else created,
            background=background,
            data=images,
            output_format=output_format,
            quality=quality,
            size=size,
            usage=ImageUsage(
                input_tokens=0,
                input_tokens_details=ImageUsageInputTokensDetails(image_tokens=0, text_tokens=0),
                output_tokens=0,
                total_tokens=0,
            ) if usage is None else usage,
        )
        if hidden_params is not None:
            self._hidden_params = dict(hidden_params)

    @override
    def model_dump(
        self, *, mode: Literal["python", "json"] = "python",
        include: Selection | None = None, exclude: Selection | None = None,
        exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue:
        dumped = super().model_dump(
            mode=mode, include=top_fields(include, is_include=True),
            exclude=top_fields(exclude, is_include=False),
            exclude_unset=exclude_unset, exclude_none=exclude_none,
        )
        match dumped:
            case dict() if "data" in dumped:
                dumped["data"] = self._serialize_image_data(
                    mode=mode, include=data_selector(include),
                    exclude=data_selector(exclude), exclude_unset=exclude_unset,
                    exclude_none=exclude_none,
                )
            case _other:
                pass
        return dumped

    def _serialize_image_data(
        self, *, mode: Literal["python", "json"], include: FieldSelector | None,
        exclude: FieldSelector | None, exclude_unset: bool, exclude_none: bool,
    ) -> list[JsonValue]:
        images = self.data
        count = len(images)
        serialized: list[JsonValue] = []
        for index, image in enumerate(images):
            include_keep, include_item = _nested_selector(include, index, count, is_include=True)
            exclude_keep, exclude_item = _nested_selector(exclude, index, count, is_include=False)
            if not include_keep or not exclude_keep:
                continue
            serialized.append(image.model_dump(
                mode=mode,
                include=image_fields(include_item, is_include=True),
                exclude=image_fields(exclude_item, is_include=False),
                exclude_unset=exclude_unset, exclude_none=exclude_none,
            ))
        return serialized


def image_object(image: StoredValue) -> StoredValue:
    match image:
        case ImageObject():
            return image
        case Mapping():
            return ImageObject(**image)
        case ModelObject():
            dumped = image.model_dump()
            match dumped:
                case Mapping():
                    return ImageObject(**dumped)
                case _other:
                    return dumped
        case _other:
            return image


def is_field_mapping(
    selector: FieldSelector,
) -> TypeGuard[Mapping[int, NestedSelector] | Mapping[str, NestedSelector]]:
    match selector:
        case Mapping():
            return True
        case _other:
            return False


def is_item_selector(selector: FieldSelector) -> TypeGuard[SelectorItems]:
    match selector:
        case list() | set() | frozenset() | tuple():
            return True
        case _other:
            return False


def is_selection_mapping(selector: Selection) -> TypeGuard[Mapping[str, FieldSelector]]:
    match selector:
        case Mapping():
            return True
        case _other:
            return False


def _nested_selector(
    selector: FieldSelector | None, index: int, count: int, *, is_include: bool,
) -> tuple[bool, FieldSelector | None]:
    if selector is None:
        return True, None
    if is_field_mapping(selector):
        return mapping_selector(selector, index, count, is_include=is_include)
    if is_item_selector(selector):
        return collection_selector(selector, index, count, is_include=is_include)
    return True, selector


def mapping_selector(
    selector: Mapping[int, NestedSelector] | Mapping[str, NestedSelector],
    index: int, count: int, *, is_include: bool,
) -> tuple[bool, FieldSelector | None]:
    found, value = mapping_value(selector, index)
    if not found:
        found, value = mapping_value(selector, index - count)
    if not found:
        found, value = mapping_value(selector, "__all__")
    keep = value is not None if is_include else value is not True
    if value is True or value is None:
        return keep, None
    return keep, value


def collection_selector(
    selector: SelectorItems, index: int, count: int, *, is_include: bool,
) -> tuple[bool, FieldSelector | None]:
    indexes: list[int] = []
    for item in selector:
        match item:
            case int():
                indexes.append(item)
            case _other:
                return True, selector
    addressed = index in indexes or index - count in indexes
    if is_include:
        return addressed, None
    return not addressed, None


def mapping_value(
    selector: Mapping[int, NestedSelector] | Mapping[str, NestedSelector], key: int | str,
) -> tuple[bool, NestedSelector | None]:
    for existing, value in selector.items():
        match existing:
            case int() if existing == key:
                return True, value
            case str() if existing == key:
                return True, value
            case _other:
                continue
    return False, None


def image_fields(
    selector: FieldSelector | None, *, is_include: bool,
) -> Collection[str] | None:
    if selector is None:
        return None
    if is_field_mapping(selector):
        names: set[str] = set()
        for key, value in selector.items():
            match key:
                case str() if is_include or value is True:
                    names.add(key)
                case _other:
                    continue
        return names
    if is_item_selector(selector):
        names = set()
        for item in selector:
            match item:
                case str():
                    names.add(item)
                case _other:
                    continue
        return names
    return None


def data_selector(selector: Selection | None) -> FieldSelector | None:
    if selector is None:
        return None
    if is_selection_mapping(selector):
        if "data" in selector:
            return selector["data"]
        return None
    return True if "data" in selector else None


def top_fields(selector: Selection | None, *, is_include: bool) -> set[str] | None:
    if selector is None:
        return None
    if is_selection_mapping(selector):
        return {key for key, value in selector.items() if is_include or value is True}
    return set(selector)
