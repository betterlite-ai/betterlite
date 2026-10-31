import logging
import time
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest import mock

import httpx
from litellm.litellm_core_utils.get_blog_posts import GetBlogPosts

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class WarningCapture(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.warnings: list[Json] = []

    def filter(self, record: logging.LogRecord) -> bool:
        subject = record.args[0] if isinstance(record.args, tuple) and record.args else None
        self.warnings.append({"subject": str(subject), "message": record.getMessage()})
        return True


def responder_for(response: Json, url: str) -> Callable[..., httpx.Response]:
    if not isinstance(response, dict):
        raise ValueError("response must be an object")
    if response.get("error", False):
        def raise_connect_error(*_args: object, **_kwargs: object) -> httpx.Response:
            raise httpx.ConnectError("mock transport failure")

        return raise_connect_error
    status = response.get("status")
    text = response.get("text", "")
    if not isinstance(status, int) or not isinstance(text, str):
        raise ValueError("response.status must be an int and response.text must be a string")

    def build_response(*_args: object, **_kwargs: object) -> httpx.Response:
        return httpx.Response(status_code=status, text=text, request=httpx.Request("GET", url))

    return build_response


def load_local_blog_posts() -> Json:
    return cast(Json, GetBlogPosts.load_local_blog_posts())


def fetch_rss_feed(input_value: dict[str, Json]) -> Json:
    url = input_value["url"]
    timeout = input_value.get("timeout", 5)
    response = input_value["response"]
    if not isinstance(url, str) or not isinstance(timeout, int):
        raise ValueError("url must be a string and timeout must be an int")
    with mock.patch.object(httpx, "get", side_effect=responder_for(response, url)):
        return GetBlogPosts.fetch_rss_feed(url, timeout=timeout)


def parse_rss_to_posts(input_value: dict[str, Json]) -> Json:
    xml = input_value["xml"]
    max_posts = input_value.get("max_posts", 1)
    if not isinstance(xml, str) or not isinstance(max_posts, int):
        raise ValueError("xml must be a string and max_posts must be an int")
    return cast(Json, GetBlogPosts.parse_rss_to_posts(xml, max_posts))


def validate_blog_posts(input_value: dict[str, Json]) -> Json:
    posts = input_value["posts"]
    if not isinstance(posts, list):
        raise ValueError("posts must be a list")
    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.filters.insert(0, capture)
    try:
        valid = GetBlogPosts.validate_blog_posts(cast(list[dict[str, str]], posts))
    finally:
        logger.removeFilter(capture)
    return {"valid": valid, "warnings": capture.warnings}


def get_blog_posts(input_value: dict[str, Json]) -> Json:
    url = input_value["url"]
    now = input_value["now"]
    response = input_value["response"]
    cached_posts = input_value.get("cached_posts")
    last_fetch_time = input_value.get("last_fetch_time", 0)
    if not isinstance(url, str) or not isinstance(now, int | float):
        raise ValueError("url must be a string and now must be a number")
    if cached_posts is not None and not isinstance(cached_posts, list):
        raise ValueError("cached_posts must be a list or null")
    if not isinstance(last_fetch_time, int | float):
        raise ValueError("last_fetch_time must be a number")

    GetBlogPosts._cached_posts = cast(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        list[dict[str, str]] | None, cached_posts
    )
    GetBlogPosts._last_fetch_time = float(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        last_fetch_time
    )

    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.filters.insert(0, capture)
    try:
        with (
            mock.patch.object(time, "time", return_value=float(now)),
            mock.patch.object(httpx, "get", side_effect=responder_for(response, url)),
        ):
            posts = GetBlogPosts.get_blog_posts(url=url)
    finally:
        logger.removeFilter(capture)
    return {"posts": cast(list[Json], posts), "warnings": capture.warnings}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "load_local_blog_posts":
        return load_local_blog_posts()
    if function == "fetch_rss_feed":
        return fetch_rss_feed(input_value)
    if function == "parse_rss_to_posts":
        return parse_rss_to_posts(input_value)
    if function == "validate_blog_posts":
        return validate_blog_posts(input_value)
    if function == "get_blog_posts":
        return get_blog_posts(input_value)
    raise ValueError(f"unknown blog_feed function: {function}")
