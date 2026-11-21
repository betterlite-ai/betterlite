import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

from fastapi import HTTPException
from litellm.proxy import proxy_server
from litellm.proxy._types import ProxyException
from litellm.proxy.auth import auth_exception_handler, auth_utils
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message, Scope

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
CLIENT_PORT: Final = 1234
MarkerName: TypeAlias = Literal["absent", "not_true", "exact_true"]


class BytesToMbCall(TypedDict):
    function: Literal["bytes_to_mb"]
    values: list[int]


class IpRequest(TypedDict):
    forwarded_for: str | None
    client_host: str | None
    use_x_forwarded_for: bool
    allowed_ips: list[str] | None


class IpCall(TypedDict):
    function: Literal["ip"]
    requests: list[IpRequest]


class IpAnswer(TypedDict):
    ip: str | None
    allowed: bool
    passed_in_ip: str | None


class ClassifyRow(TypedDict):
    kind: Literal["http", "proxy", "other"]
    code: str | None
    status_code: int
    marker: MarkerName
    message: str


class ClassifyCall(TypedDict):
    function: Literal["classify"]
    rows: list[ClassifyRow]


class MarkRow(TypedDict):
    is_invalid_virtual_key: bool
    code: str
    message: str
    openai_code: str | None
    headers: dict[str, str]


class MarkCall(TypedDict):
    function: Literal["mark"]
    rows: list[MarkRow]


class MarkAnswer(TypedDict):
    same_object: bool
    classified: bool
    type: str
    param: str | None
    code: str
    openai_code: str | int | None
    headers: dict[str, str]


class PreDbRequest(TypedDict):
    allowed_ips: list[str] | None
    allowed_routes: list[str] | None
    use_x_forwarded_for: bool
    route: str
    forwarded_for: str | None
    client_host: str | None
    content_length: str | None
    max_request_size_mb: float | None
    body: dict[str, JsonValue]


class PreDbCall(TypedDict):
    function: Literal["pre_db"]
    requests: list[PreDbRequest]


class PreDbAnswer(TypedDict):
    ok: bool
    status: int | None
    type: str | None
    param: str | None
    code: str | None
    message: str | None


Call: TypeAlias = BytesToMbCall | IpCall | ClassifyCall | MarkCall | PreDbCall


class Input(TypedDict):
    calls: list[Call]


class EmptyBody:
    async def __call__(self) -> Message:
        message: Message = {"type": "http.request", "body": b"", "more_body": False}
        return message


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return _JSON.validate_python(answers)


def answer(call: Call) -> JsonValue:
    if call["function"] == "bytes_to_mb":
        return bytes_answer(call)
    if call["function"] == "ip":
        return ip_answer(call)
    if call["function"] == "classify":
        return classify_answer(call)
    if call["function"] == "mark":
        return mark_answer(call)
    return pre_db_answer(call)


def bytes_answer(call: BytesToMbCall) -> JsonValue:
    sizes: list[float] = [auth_utils.bytes_to_mb(value) for value in call["values"]]  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L942-L946
    return _JSON.validate_python(sizes)


def ip_answer(call: IpCall) -> JsonValue:
    rows: list[JsonValue] = [ip_row(request) for request in call["requests"]]
    return _JSON.validate_python(rows)


def ip_row(request: IpRequest) -> JsonValue:
    http_request = build_request(
        route="/",
        forwarded_for=request["forwarded_for"],
        client_host=request["client_host"],
        content_length=None,
    )
    use_forwarded = request["use_x_forwarded_for"]
    allowed, passed_in_ip = auth_utils._check_valid_ip(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L90-L108
        allowed_ips=request["allowed_ips"],
        request=http_request,
        use_x_forwarded_for=use_forwarded,
    )
    ip = auth_utils._get_request_ip_address(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L78-L87
        request=http_request,
        use_x_forwarded_for=use_forwarded,
    )
    return _JSON.validate_python(IpAnswer(ip=ip, allowed=allowed, passed_in_ip=passed_in_ip))


def classify_answer(call: ClassifyCall) -> JsonValue:
    answers: list[JsonValue] = [
        auth_utils.is_invalid_virtual_key_error(classified_error(row))
        for row in call["rows"]
    ]
    return _JSON.validate_python(answers)


def classified_error(row: ClassifyRow) -> BaseException:
    match row["kind"]:
        case "http":
            error: BaseException = HTTPException(
                status_code=row["status_code"], detail=row["message"]
            )
            code = row["code"]
            if code is not None:
                error.code = code  # type: ignore[attr-defined]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L53-L55
        case "proxy":
            error = ProxyException(
                message=row["message"],
                type="auth_error",
                param="None",
                code=row["code"],
            )
        case "other":
            return ValueError(row["message"])
    stamp_marker(error, row["marker"])
    return error


def stamp_marker(error: BaseException, marker: MarkerName) -> None:
    if marker == "absent":
        return
    error._litellm_invalid_virtual_key_error = marker == "exact_true"  # type: ignore[attr-defined]  # noqa: SLF001  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L74


def mark_answer(call: MarkCall) -> JsonValue:
    answers: list[JsonValue] = [mark_row(row) for row in call["rows"]]
    return _JSON.validate_python(answers)


def mark_row(row: MarkRow) -> JsonValue:
    exception = ProxyException(
        message=row["message"],
        type="auth_error",
        param="None",
        code=row["code"],
        headers=row["headers"].copy(),
        openai_code=row["openai_code"],
    )
    marked = auth_utils.mark_invalid_virtual_key_error(exception, row["is_invalid_virtual_key"])
    return _JSON.validate_python(
        MarkAnswer(
            same_object=marked is exception,
            classified=auth_utils.is_invalid_virtual_key_error(marked),
            type=marked.type,
            param=marked.param,
            code=marked.code,
            openai_code=marked.openai_code,
            headers=marked.headers,
        )
    )


def pre_db_answer(call: PreDbCall) -> JsonValue:
    answers: list[JsonValue] = [pre_db_row(request) for request in call["requests"]]
    return _JSON.validate_python(answers)


def pre_db_row(request: PreDbRequest) -> JsonValue:
    http_request = build_request(
        route=request["route"],
        forwarded_for=request["forwarded_for"],
        client_host=request["client_host"],
        content_length=request["content_length"],
    )
    # the size and route checks ask for an enterprise license, and no model group carries a grant
    proxy_server.general_settings = general_settings(request)
    proxy_server.premium_user = True
    proxy_server.llm_router = None
    try:
        asyncio.run(
            auth_utils.pre_db_read_auth_checks(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L562-L618
                request=http_request,
                request_data=request["body"],
                route=request["route"],
            )
        )
    except Exception as error:  # noqa: BLE001 - the pre-DB helper reports every rejection it raises  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L562-L618
        return verdict(auth_exception_handler._as_proxy_exception(error))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
    return verdict(None)


def general_settings(request: PreDbRequest) -> dict[str, JsonValue]:
    settings: dict[str, JsonValue] = {"use_x_forwarded_for": request["use_x_forwarded_for"]}
    allowed_ips = request["allowed_ips"]
    if allowed_ips is not None:
        settings["allowed_ips"] = _JSON.validate_python(allowed_ips)
    allowed_routes = request["allowed_routes"]
    if allowed_routes is not None:
        settings["allowed_routes"] = _JSON.validate_python(allowed_routes)
    max_size = request["max_request_size_mb"]
    if max_size is not None:
        settings["max_request_size_mb"] = max_size
    return settings


def verdict(exception: ProxyException | None) -> JsonValue:
    if exception is None:
        return _JSON.validate_python(
            PreDbAnswer(ok=True, status=None, type=None, param=None, code=None, message=None)
        )
    return _JSON.validate_python(
        PreDbAnswer(
            ok=False,
            status=int(exception.code),
            type=exception.type,
            param=exception.param,
            code=exception.code,
            message=exception.message,
        )
    )


def build_request(
    route: str,
    forwarded_for: str | None,
    client_host: str | None,
    content_length: str | None,
) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    if forwarded_for is not None:
        headers.append((b"x-forwarded-for", forwarded_for.encode()))
    if content_length is not None:
        headers.append((b"content-length", content_length.encode()))
    client: tuple[str, int] | None = None if client_host is None else (client_host, CLIENT_PORT)
    scope: Scope = {"type": "http", "path": route, "headers": headers, "client": client}
    return Request(scope=scope, receive=EmptyBody())
