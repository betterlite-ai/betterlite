from pydantic import JsonValue

from .common import ResponsesApi, ResponsesConfig, UrlCase, guarded


def url_of(config: ResponsesApi, case: UrlCase) -> dict[str, JsonValue]:
    url = config.get_complete_url(case.get("api_base"), {})
    return {"url": url}


def responses_url_values(cases: list[UrlCase]) -> list[JsonValue]:
    config: ResponsesApi = ResponsesConfig()
    return [guarded(url_of, config, case) for case in cases]
