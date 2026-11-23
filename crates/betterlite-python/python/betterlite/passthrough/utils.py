from collections.abc import Mapping
from typing import Final, Protocol

from pydantic import TypeAdapter

from betterlite import _native

QUERY: Final[TypeAdapter[dict[str, str | list[str]]]] = TypeAdapter(dict[str, str | list[str]])
HEADERS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])


# the httpx.URL a caller hands over; the merge reads the URL's text
class Url(Protocol):
    def __str__(self) -> str: ...


class BasePassthroughUtils:
    @staticmethod
    def get_merged_query_parameters(
        existing_url: Url,
        request_query_params: Mapping[str, str | list[str]],
        default_query_params: Mapping[str, str | list[str]] | None = None,
    ) -> dict[str, str | list[str]]:
        request = QUERY.dump_json(dict(request_query_params)).decode()
        defaults = (
            None
            if default_query_params is None
            else QUERY.dump_json(dict(default_query_params)).decode()
        )
        merged = _native.merge_query_parameters(str(existing_url), request, defaults)
        return QUERY.validate_json(merged)

    @staticmethod
    def forward_headers_from_request(
        request_headers: Mapping[str, str],
        headers: Mapping[str, str],
        forward_headers: bool | None = False,
    ) -> dict[str, str]:
        request = HEADERS.dump_json(dict(request_headers)).decode()
        provided = HEADERS.dump_json(dict(headers)).decode()
        forwarded = _native.forward_headers_from_request(
            request, provided, forward_headers=forward_headers is True
        )
        return HEADERS.validate_json(forwarded)


class CommonUtils:
    @staticmethod
    # ast-grep-ignore: no-long-names, no-one-line-functions
    def encode_bedrock_runtime_modelid_arn(endpoint: str) -> str:
        return _native.encode_bedrock_runtime_modelid_arn(endpoint)
