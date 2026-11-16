from pydantic import JsonValue, TypeAdapter

from .common import CHAT, HeadersCase, Input, Object, cases_of, guarded


def headers_of(case: HeadersCase) -> Object:
    optional: Object = dict(case["optional_params"])
    user_agent = case["user_agent"]
    litellm_params: Object = {} if user_agent is None else {"user_agent": user_agent}
    given = case["headers"]
    headers: Object = {} if given is None else dict(given)
    returned = CHAT.validate_environment(
        headers,
        case["model"],
        [],
        optional,
        litellm_params,
        api_key=case["api_key"],
        api_base=case["api_base"],
    )
    return {"headers": returned, "optional_params": optional}


def headers_value(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[HeadersCase]).validate_python(cases_of(input_value))
    return [guarded(headers_of, case) for case in cases]
