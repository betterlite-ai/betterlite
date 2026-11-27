from typing import Literal, NotRequired, Protocol, TypedDict

from pydantic import JsonValue


class Call(TypedDict):
    purpose: NotRequired[str | None]
    file_id: NotRequired[str | None]
    status: NotRequired[int]
    body: NotRequired[str]
    headers: NotRequired[list[tuple[str, str]]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class ListingCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class DeleteResponseCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    additional_args: NotRequired[dict[str, JsonValue] | None]


class RetrieveCall(TypedDict):
    file_id: str
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]


class ListRequests(TypedDict):
    operation: Literal["list_request"]
    requests: list[Call]


class NextRequests(TypedDict):
    operation: Literal["next_request"]
    next: list[Call]


class ListResponses(TypedDict):
    operation: Literal["list_response"]
    listings: list[ListingCall]


class DeleteRequests(TypedDict):
    operation: Literal["delete_request"]
    deletes: list[Call]


class ContentRequests(TypedDict):
    operation: Literal["content_request"]
    contents: list[Call]


class DeleteResponses(TypedDict):
    operation: Literal["delete_response"]
    delete_responses: list[DeleteResponseCall]


class Retrieves(TypedDict):
    operation: Literal["retrieve"]
    retrieves: list[RetrieveCall]


Input = (
    ListRequests
    | NextRequests
    | ListResponses
    | DeleteRequests
    | ContentRequests
    | DeleteResponses
    | Retrieves
)

RequestOperation = Literal["list_request", "next_request", "delete_request", "content_request"]


# LiteLLM declares the retrieval request's query as a bare dict, so the protocol names the shape the case observes
class RetrievalRequest(Protocol):
    def transform_retrieve_file_request(
        self,
        file_id: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> tuple[str, dict[str, str]]: ...
