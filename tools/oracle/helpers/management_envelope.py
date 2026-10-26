from typing import Literal, NotRequired, TypedDict

from litellm.types.proxy.management_endpoints.management_v1 import (
    FacetListResponse,
    ListLinks,
    ListMeta,
    ListResponse,
    PageLinks,
    PageMeta,
    ProblemDetail,
    ResourceResponse,
)


class ProblemInput(TypedDict):
    function: Literal["problem"]
    type: str
    title: str
    status: int
    detail: str
    document: NotRequired[bool]
    allowed: NotRequired[list[str] | None]


class FacetInput(TypedDict):
    function: Literal["facet"]
    data: list[str]
    page: int
    page_size: int
    has_more: bool
    self: str
    prev: str | None
    next: str | None


class ListInput(TypedDict):
    function: Literal["list"]
    data: list[object]
    total_count: int
    page: int
    page_size: int
    total_pages: int
    self_link: str
    first: str
    prev: str | None
    next: str | None
    last: str


class ResourceInput(TypedDict):
    function: Literal["resource"]
    data: object


Input = ProblemInput | FacetInput | ListInput | ResourceInput


def run(input_value: Input) -> object:
    match input_value["function"]:
        case "problem":
            problem = ProblemDetail.model_validate(input_value)
            return problem.model_dump(
                mode="json", by_alias=True, exclude_none=input_value.get("document", False)
            )
        case "facet":
            response = FacetListResponse(
                data=input_value["data"],
                meta=PageMeta(
                    page=input_value["page"],
                    page_size=input_value["page_size"],
                    has_more=input_value["has_more"],
                ),
                links=PageLinks.model_validate(input_value),
            )
            return response.model_dump(mode="json", by_alias=True)
        case "list":
            response = ListResponse(
                data=input_value["data"],
                meta=ListMeta(
                    total_count=input_value["total_count"],
                    page=input_value["page"],
                    page_size=input_value["page_size"],
                    total_pages=input_value["total_pages"],
                ),
                links=ListLinks.model_validate(input_value),
            )
            return response.model_dump(mode="json", by_alias=True)
        case "resource":
            response = ResourceResponse[object].model_validate({"data": input_value["data"]})
            return response.model_dump(mode="json", by_alias=True)
