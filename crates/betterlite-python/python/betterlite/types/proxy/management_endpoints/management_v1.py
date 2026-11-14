from collections.abc import Sequence
from typing import Annotated, Generic, TypeVar

from pydantic import ConfigDict, Field
from typing_extensions import override

from betterlite.types.utils import ResponseObject
from betterlite.types.utils.base import InitValue

TOut = TypeVar("TOut")


class ProblemDetail(ResponseObject):
    type: str
    title: str
    status: int
    detail: str
    allowed: list[str] | None = None


class _SelfLinkModel(ResponseObject):
    @override
    def __init__(self, /, **data: InitValue) -> None:
        # the "/" lets an alias named "self" arrive in data instead of binding to self
        if "self" in data:
            data["self_link"] = data.pop("self")
        super().__init__(**data)


class PageLinks(_SelfLinkModel):
    model_config = ConfigDict(populate_by_name=True)

    self_link: Annotated[str, Field(alias="self")]
    prev: str | None = None
    next: str | None = None


class PageMeta(ResponseObject):
    page: int
    page_size: int
    has_more: bool


class FacetListResponse(ResponseObject):
    data: Sequence[str]
    meta: PageMeta
    links: PageLinks


class ListMeta(ResponseObject):
    total_count: int
    page: int
    page_size: int
    total_pages: int


class ListLinks(_SelfLinkModel):
    model_config = ConfigDict(populate_by_name=True)

    self_link: Annotated[str, Field(alias="self")]
    first: str
    prev: str | None = None
    next: str | None = None
    last: str


class ResourceResponse(ResponseObject, Generic[TOut]):
    data: TOut


class ListResponse(ResponseObject, Generic[TOut]):
    data: list[TOut]
    meta: ListMeta
    links: ListLinks
