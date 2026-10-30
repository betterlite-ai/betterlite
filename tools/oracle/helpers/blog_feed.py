from __future__ import annotations

import os
from typing import TypedDict, cast
from xml.etree.ElementTree import ParseError

from litellm.litellm_core_utils.get_blog_posts import GetBlogPosts

BLOG_POSTS_TTL_SECONDS = 3600
HTTP_SUCCESS_MIN = 200
HTTP_SUCCESS_MAX = 299


class Response(TypedDict, total=False):
    status: int
    text: str
    error: bool


def fetch(response: Response, url: str, timeout: int) -> str:
    if response.get("error", False):
        raise RuntimeError(f"HTTP request to {url} failed after {timeout} seconds")
    status = response.get("status", 0)
    if status < HTTP_SUCCESS_MIN or status > HTTP_SUCCESS_MAX:
        raise RuntimeError(f"RSS response returned HTTP status {status}")
    return response.get("text", "")


def parse(xml: str, max_posts: int) -> list[dict[str, str]]:
    return GetBlogPosts.parse_rss_to_posts(xml, max_posts)


def get_posts(data: dict[str, object]) -> list[dict[str, str]]:
    if os.getenv("LITELLM_LOCAL_BLOG_POSTS", "").lower() == "true":
        return GetBlogPosts.load_local_blog_posts()
    cached = data.get("cached_posts")
    now = float(cast(float, data["now"]))
    last = float(cast(float, data.get("last_fetch_time", 0)))
    if isinstance(cached, list) and now - last < BLOG_POSTS_TTL_SECONDS:
        return cast(list[dict[str, str]], cached)
    try:
        posts = parse(fetch(cast(Response, data["response"]), str(data["url"]), 5), 1)
    except (ParseError, RuntimeError, ValueError):
        return GetBlogPosts.load_local_blog_posts()
    if not posts:
        return GetBlogPosts.load_local_blog_posts()
    return posts


def run(data: dict[str, object]) -> object:
    function = data["function"]
    if function == "load_local_blog_posts":
        return GetBlogPosts.load_local_blog_posts()
    if function == "fetch_rss_feed":
        response = data["response"]
        timeout = int(cast(int, data.get("timeout", 5)))
        return fetch(cast(Response, response), str(data["url"]), timeout)
    if function == "parse_rss_to_posts":
        return parse(str(data["xml"]), int(cast(int, data.get("max_posts", 1))))
    if function == "validate_blog_posts":
        posts = data["posts"]
        return isinstance(posts, list) and len(cast(list[object], posts)) > 0
    if function == "get_blog_posts":
        return get_posts(data)
    raise ValueError("unknown blog feed function")
