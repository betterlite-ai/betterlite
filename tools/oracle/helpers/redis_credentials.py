from typing import TypedDict

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    function: str
    service_account: str
    cache_name: str
    serverless: bool
    client_id: str | None
    tenant_id: str | None
    client_secret: str | None


def run(data: Input) -> Json:
    match data["function"]:
        case "gcp_token_caching":
            account = data["service_account"]
            return {
                "provider": "gcp",
                "service_account_configured": bool(account),
                "token_ttl_seconds": 3300,
            }
        case "elasticache_sig_generation":
            query = (
                "Action=connect&ResourceType=ServerlessCache"
                if data.get("serverless", False)
                else "Action=connect"
            )
            return {
                "provider": "aws",
                "cache_name": data["cache_name"].lower(),
                "token_ttl_seconds": 900,
                "query": query,
            }
        case "azure_credential_selection":
            if data.get("client_id") and data.get("tenant_id") and data.get("client_secret"):
                credential = "client_secret"
            elif data.get("client_id"):
                credential = "managed_identity"
            else:
                credential = "default"
            return {"provider": "azure", "credential": credential}
        case _:
            raise ValueError("unknown redis credentials function")
