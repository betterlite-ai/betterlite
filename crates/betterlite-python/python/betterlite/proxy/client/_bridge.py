from betterlite._native import ProxyClientNative

_native = ProxyClientNative()

redact_string = _native.redact_text
get_gateway_api_key = _native.gateway_api_key
