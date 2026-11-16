import os
from collections.abc import Generator
from contextlib import contextmanager
from typing import Final

from pydantic import JsonValue

from .models import Object

MODEL: Final = "openai.gpt-oss-120b"
API_BASE: Final = "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1/responses"
ENVIRONMENT_KEYS: Final = (
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_BEDROCK_RUNTIME_ENDPOINT",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_REGION_NAME",
)
SIGNED_SCHEME: Final = "AWS4-HMAC-SHA256 "
TIME_DEPENDENT: Final = ("Authorization", "X-Amz-Date")


class FixtureRefusalError(Exception):
    def __init__(self, url: str) -> None:
        super().__init__(f"no fixture for {url}")


class Fixture:
    def __init__(self, data_urls: dict[str, str]) -> None:
        self.data_urls = data_urls
        self.fetched: list[JsonValue] = []

    def fetch(self, url: str) -> str:
        self.fetched.append(url)
        data_url = self.data_urls.get(url)
        if data_url is None:
            raise FixtureRefusalError(url)
        return data_url

    async def fetch_async(self, url: str) -> str:
        self.fetched.append(url)
        data_url = self.data_urls.get(url)
        if data_url is None:
            raise FixtureRefusalError(url)
        return data_url


@contextmanager
def environment(values: dict[str, str] | None) -> Generator[None]:
    given = {} if values is None else values
    saved = {name: os.environ.pop(name, None) for name in ENVIRONMENT_KEYS}
    os.environ.update(given)
    try:
        yield
    finally:
        for name in ENVIRONMENT_KEYS:
            os.environ.pop(name, None)
        os.environ.update({name: text for name, text in saved.items() if text is not None})


def signed_summary(headers: dict[str, str]) -> Object:
    # The signing time and signature change on every run, so only their shape is compared
    authorization = headers["Authorization"].removeprefix(SIGNED_SCHEME)
    _, _, rest = authorization.partition("Credential=")
    credential, _, rest = rest.partition(", SignedHeaders=")
    signed_headers, _, _signature = rest.partition(", Signature=")
    access_key, _date, region, service, _scope = credential.split("/")
    plain: Object = {name: text for name, text in headers.items() if name not in TIME_DEPENDENT}
    return {
        "access_key": access_key,
        "region": region,
        "service": service,
        "signed_headers": signed_headers,
        "headers": plain,
    }
