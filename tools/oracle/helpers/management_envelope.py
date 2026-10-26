from typing import Any, Literal, TypedDict

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
    function = input_value["function"]
    if function == "problem":
        problem = ProblemDetail.model_validate(input_value)
        return problem.model_dump(
            mode="json", by_alias=True, exclude_none=input_value.get("document", False)
        )
    if function == "facet":
        response = FacetListResponse(
            data=input_value["data"],
            meta=PageMeta(
                page=input_value["page"], page_size=input_value["page_size"],
                has_more=input_value["has_more"],
            ),
            links=PageLinks.model_validate(input_value),
        )
        return response.model_dump(mode="json", by_alias=True)
    if function == "list":
        response = ListResponse(
            data=input_value["data"],
            meta=ListMeta(
                total_count=input_value["total_count"], page=input_value["page"],
                page_size=input_value["page_size"], total_pages=input_value["total_pages"],
            ),
            links=ListLinks.model_validate(input_value),
        )
        return response.model_dump(mode="json", by_alias=True)
    return {"data": input_value["data"]}
