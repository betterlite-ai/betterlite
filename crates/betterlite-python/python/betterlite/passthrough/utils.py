from collections.abc import AsyncIterable, Iterable, Mapping, Sequence
from http.cookiejar import CookieJar
from typing import Final, Protocol, TypeAlias, TypedDict
from urllib.parse import parse_qsl

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypeIs, override

from betterlite import _native
from betterlite._completion.arguments import SessionLike
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue, Readable
from betterlite.types.llms.openai.core.aliases import FileTypes

QUERY: Final[TypeAdapter[dict[str, str | list[str]]]] = TypeAdapter(dict[str, str | list[str]])
HEADERS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])

ScalarParam: TypeAlias = str | int | float | bool | None
ParamValue: TypeAlias = ScalarParam | list[ScalarParam]
Content: TypeAlias = (
    str | bytes | bytearray | memoryview | Readable | Iterable[bytes] | AsyncIterable[bytes]
)
Files: TypeAlias = Mapping[str, FileTypes] | Sequence[tuple[str, FileTypes]]
Params: TypeAlias = str | bytes | Mapping[str, ParamValue] | Sequence[tuple[str, ParamValue]]
Cookies: TypeAlias = CookieJar | Mapping[str, str] | Sequence[tuple[str, str]]


# the httpx.URL a caller hands over; the merge reads the URL's text
class Url(Protocol):
    @override
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
        forward_headers: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/passthrough/utils.py#L62
    ) -> dict[str, str]:
        forwarded = _native.forward_headers_from_request(
            HEADERS.dump_json(dict(request_headers)).decode(),
            HEADERS.dump_json(dict(headers)).decode(),
            forward_headers=forward_headers is True,
        )
        return HEADERS.validate_json(forwarded)


class CommonUtils:
    @staticmethod
    # ast-grep-ignore: no-long-names, no-one-line-functions
    def encode_bedrock_runtime_modelid_arn(endpoint: str) -> str:
        return _native.encode_bedrock_runtime_modelid_arn(endpoint)


class RouteValues(TypedDict):
    method: str
    endpoint: str
    model: str
    custom_llm_provider: str | None
    api_base: str | None
    api_key: str | None
    request_query_params: dict[str, JsonValue] | None
    request_headers: dict[str, str] | None
    content: bytes | None
    data: dict[str, CallValue] | None
    files: Files | None
    json: JsonValue
    params: Params | None
    cookies: Cookies | None
    client: SessionLike | None
    kwargs: Mapping[str, CallValue]


def parameter_text(value: ScalarParam) -> str:
    if value is True:
        return "true"
    if value is False:
        return "false"
    if value is None:
        return ""
    return str(value)


def parameter_entries(name: str, value: ParamValue) -> list[CallValue]:
    match value:
        case list() as items:
            return [[name, parameter_text(item)] for item in items]
        case str() | bool() | int() | float() | None:
            return [[name, parameter_text(value)]]


def is_query_text(values: Params) -> TypeIs[str | bytes]:
    match values:
        case str() | bytes():
            return True
        case object():
            return False


def is_query_mapping(values: Params) -> TypeIs[Mapping[str, ParamValue]]:
    match values:
        case Mapping():
            return True
        case object():
            return False


def parameter_pairs(params: Params) -> list[CallValue]:
    if is_query_text(params):
        match params:
            case str() as text:
                return [[name, value] for name, value in parse_qsl(text, keep_blank_values=True)]
            case bytes() as data:
                decoded = data.decode()
                return [[name, value] for name, value in parse_qsl(decoded, keep_blank_values=True)]
    if is_query_mapping(params):
        return [pair for name, value in params.items() for pair in parameter_entries(name, value)]
    return [pair for name, value in params for pair in parameter_entries(name, value)]


def cookie_pairs(cookies: Cookies) -> list[CallValue]:
    match cookies:
        case CookieJar():
            return [[cookie.name, cookie.value] for cookie in cookies]
        case Mapping():
            return [[name, value] for name, value in cookies.items()]
        case Sequence():
            return [[name, value] for name, value in cookies]


def file_fields(files: Files) -> dict[str, CallValue]:
    pairs: list[tuple[str, FileTypes]] = []
    match files:
        case Mapping() as mapping:
            pairs.extend(mapping.items())
        case Sequence() as sequence:
            pairs.extend(sequence)
    grouped: dict[str, list[CallValue]] = {}
    for field, content in pairs:
        grouped.setdefault(field, []).append(content)
    return {field: items[0] if len(items) == 1 else items for field, items in grouped.items()}


def content_bytes(content: Content) -> bytes:
    match content:
        case str():
            return content.encode()
        case bytes():
            return content
        case bytearray():
            return bytes(content)
        case memoryview():
            return content.tobytes()
        case Readable():
            return content.read()
        case AsyncIterable():
            raise TypeError("the synchronous route takes content that is iterable, not async")
        case object():
            return b"".join(content)


async def acontent_bytes(content: Content) -> bytes:
    match content:
        case AsyncIterable():
            return b"".join([chunk async for chunk in content])
        case object():
            return content_bytes(content)


def request_values(route: RouteValues) -> dict[str, CallValue]:
    provider_config = route["kwargs"].get("provider_config")
    if provider_config is not None:
        raise UnsupportedCallArgument("provider_config")
    logging_object = route["kwargs"].get("litellm_logging_obj")
    if logging_object is not None:
        raise UnsupportedCallArgument("litellm_logging_obj")
    files = route["files"]
    params = route["params"]
    cookies = route["cookies"]
    return {
        "method": route["method"],
        "endpoint": route["endpoint"],
        "model": route["model"],
        "custom_llm_provider": route["custom_llm_provider"],
        "api_base": route["api_base"],
        "api_key": route["api_key"],
        "request_query_params": route["request_query_params"],
        "request_headers": route["request_headers"],
        "content": route["content"],
        "data": route["data"],
        "files": None if files is None else file_fields(files),
        "json": route["json"],
        "params": None if params is None else parameter_pairs(params),
        "cookies": None if cookies is None else cookie_pairs(cookies),
        "client": route["client"],
        **route["kwargs"],
    }
