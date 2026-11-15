from pydantic import JsonValue

from ._transport import RequestError
from .http_client import HTTPClient


class HealthManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._http = HTTPClient(base_url=base_url, api_key=api_key, timeout=timeout)

    def get_readiness(self) -> dict[str, JsonValue]:
        reply = self._http.request("GET", "/health/readiness")
        match reply:
            case dict():
                return reply
            case list() | str() | bool() | int() | float() | None:
                raise RequestError("the readiness reply is not a JSON object")

    def get_server_version(self) -> str | None:
        match self.get_readiness().get("litellm_version"):
            case str() as version:
                return version
            case bool() | int() | float() | list() | dict() | None:
                return None
