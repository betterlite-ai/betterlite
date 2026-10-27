from typing import TypedDict

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict, total=False):
    function: str
    service_account: str
    cache_name: str
    serverless: bool
    client_id: str | None
    tenant_id: str | None
    client_secret: str | None


def run(data: Input) -> Json:
    function = data["function"]
    if function == "gcp_token_caching":
        account = data["service_account"]
        return {"provider": "gcp", "service_account_configured": bool(account), "token_ttl_seconds": 3300}
    if function == "elasticache_sig_generation":
        query = "Action=connect&ResourceType=ServerlessCache" if data.get("serverless", False) else "Action=connect"
        return {"provider": "aws", "cache_name": data["cache_name"].lower(), "token_ttl_seconds": 900, "query": query}
    if function == "azure_credential_selection":
        if data.get("client_id") and data.get("tenant_id") and data.get("client_secret"):
            credential = "client_secret"
        elif data.get("client_id"):
            credential = "managed_identity"
        else:
            credential = "default"
        return {"provider": "azure", "credential": credential}
    raise ValueError("unknown redis credentials function")
