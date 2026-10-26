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
    problem: ProblemDetail
    document: NotRequired[bool]


class FacetInput(TypedDict):
    function: Literal["facet"]
    data: list[str]
    meta: PageMeta
    links: PageLinks


class ListInput(TypedDict):
    function: Literal["list"]
    data: list[object]
    meta: ListMeta
    links: ListLinks


class ResourceInput(TypedDict):
    function: Literal["resource"]
    data: object


Input = ProblemInput | FacetInput | ListInput | ResourceInput


def run(input_value: Input) -> object:
    match input_value["function"]:
        case "problem":
            problem = ProblemDetail.model_validate(input_value["problem"])
            return problem.model_dump(
                mode="json", by_alias=True, exclude_none=input_value.get("document", False)
            )
        case "facet":
            response = FacetListResponse(
                data=input_value["data"],
                meta=PageMeta.model_validate(input_value["meta"]),
                links=PageLinks.model_validate(input_value["links"]),
            )
            return response.model_dump(mode="json", by_alias=True)
        case "list":
            response = ListResponse(
                data=input_value["data"],
                meta=ListMeta.model_validate(input_value["meta"]),
                links=ListLinks.model_validate(input_value["links"]),
            )
            return response.model_dump(mode="json", by_alias=True)
        case "resource":
            response = ResourceResponse[object].model_validate({"data": input_value["data"]})
            return response.model_dump(mode="json", by_alias=True)
