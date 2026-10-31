from litellm.proxy import proxy_server


def set_premium_user() -> None:
    proxy_server.premium_user = True
