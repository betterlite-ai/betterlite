from typing import Any, Literal, TypedDict, cast

from pydantic import BaseModel, ConfigDict, Field


class ProblemDetail(BaseModel):
    type: str
    title: str
    status: int
    detail: str
    allowed: list[str] | None = None


class PageLinks(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    self_link: str = Field(alias="self")
    prev: str | None = None
    next: str | None = None


class PageMeta(BaseModel):
    page: int
    page_size: int
    has_more: bool


class FacetListResponse(BaseModel):
    data: list[str]
    meta: PageMeta
    links: PageLinks


class ListMeta(BaseModel):
    total_count: int
    page: int
    page_size: int
    total_pages: int


class ListLinks(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    self_link: str = Field(alias="self")
    first: str
    prev: str | None = None
    next: str | None = None
    last: str


class ListResponse(BaseModel):
    data: list[Any]
    meta: ListMeta
    links: ListLinks


class Input(TypedDict, total=False):
    function: Literal["problem", "facet", "list", "resource"]
    type: str
    title: str
    status: int
    detail: str
    allowed: list[str] | None
    document: bool
    data: Any
    page: int
    page_size: int
    has_more: bool
    self: str
    self_link: str
    prev: str | None
    next: str | None
    total_count: int
    total_pages: int
    first: str
    last: str


def run(input_value: Input) -> dict[str, Any]:
    values = cast(dict[str, object], input_value)
    function = cast(str, values["function"])
    if function == "problem":
        problem = ProblemDetail.model_validate(values)
        return problem.model_dump(
            mode="json", by_alias=True,
            exclude_none=cast(bool, values.get("document", False)),
        )
    if function == "facet":
        response = FacetListResponse(
            data=cast(list[str], values["data"]),
            meta=PageMeta(
                page=cast(int, values["page"]), page_size=cast(int, values["page_size"]),
                has_more=cast(bool, values["has_more"]),
            ),
            links=PageLinks.model_validate(values),
        )
        return response.model_dump(mode="json", by_alias=True)
    if function == "list":
        response = ListResponse(
            data=cast(list[Any], values["data"]),
            meta=ListMeta(
                total_count=cast(int, values["total_count"]), page=cast(int, values["page"]),
                page_size=cast(int, values["page_size"]),
                total_pages=cast(int, values["total_pages"]),
            ),
            links=ListLinks.model_validate(values),
        )
        return response.model_dump(mode="json", by_alias=True)
    return {"data": cast(Any, values["data"])}
