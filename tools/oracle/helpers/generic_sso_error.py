import os
from typing import TypedDict

from litellm.proxy._types import ProxyException
from litellm.proxy.management_endpoints import ui_sso


class Source(TypedDict):
    status: int
    error_type: str
    param: str | None
    code: str


class Input(TypedDict):
    message: str
    source: Source
    environment: dict[str, str]


class Denial(TypedDict):
    status: int
    error_type: str
    param: str | None
    code: str


def denial_of(denial: ProxyException) -> Denial:
    return {
        "status": int(denial.code),
        "error_type": denial.type,
        "param": denial.param,
        "code": denial.code,
    }


def run(input_value: Input) -> Denial:
    source = input_value["source"]
    error = ProxyException(
        message=input_value["message"],
        type=source["error_type"],
        param=source["param"],
        code=source["code"],
    )
    headers: dict[str, str] = {}
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        ui_sso._handle_generic_sso_error(error, None, None, headers)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L1417
    except ProxyException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return denial_of(raised)
    finally:
        os.environ.clear()
        os.environ.update(saved)
