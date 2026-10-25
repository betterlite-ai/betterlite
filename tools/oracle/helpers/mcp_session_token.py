import jwt
ISSUER = "litellm-mcp-gateway"
MAX_BYTES = 4096

def run(fields: dict[str, object]) -> object:
    action = fields.get("action")
    candidate = fields.get("candidate", "")
    if action == "prefix":
        return {"session": isinstance(candidate, str) and candidate.startswith("llm_session_"), "refresh": isinstance(candidate, str) and candidate.startswith("llm_srefresh_")}
    if action == "mint":
        return mint(fields)
    if action == "open":
        return open_token(fields)
    raise ValueError("unknown session token action")
def mint(fields: dict[str, object]) -> dict[str, object]:
    refresh = fields.get("refresh") is True
    user_id, client_id, key, now = fields.get("user_id", ""), fields.get("client_id", ""), fields.get("key", ""), fields.get("now", 0)
    if client_id == "__oversized__":
        client_id = "x" * 4096
    if not all(isinstance(value, str) for value in (user_id, client_id, key)) or not isinstance(now, int) or not user_id or not client_id:
        return {"error": "session token is malformed"}
    ttl = 1209600 if refresh else 3600
    claims = {"iss": ISSUER, "iat": now, "exp": now + ttl, "jti": "x" * 22, "kind": "session_refresh" if refresh else "session", "user_id": user_id, "client_id": client_id}
    for name in ("resource_server_id", "audience", "team_id"):
        if fields.get(name) is not None:
            claims[name] = fields[name]
    token = ("llm_srefresh_" if refresh else "llm_session_") + jwt.encode(claims, key, algorithm="HS256")
    size = len(token.encode())
    if size > MAX_BYTES:
        return {"error": f"session token is too large: {size} bytes, maximum {MAX_BYTES}"}
    return {"expires_at": now + ttl, "prefix": "llm_srefresh_" if refresh else "llm_session_", "token_bytes": size}
def open_token(fields: dict[str, object]) -> dict[str, object]:
    refresh = fields.get("refresh") is True
    candidate, key, now = fields.get("candidate", ""), fields.get("key", ""), fields.get("now", 0)
    if candidate == "__oversized__":
        candidate = "llm_session_" + "x" * MAX_BYTES
    prefix = "llm_srefresh_" if refresh else "llm_session_"
    if not isinstance(candidate, str) or not candidate.startswith(prefix):
        return {"error": "candidate is not a session token"}
    if len(candidate) > MAX_BYTES or not isinstance(key, str) or len(key.encode()) < 32 or not isinstance(now, int):
        return {"error": "session token is malformed"}
    try:
        claims = jwt.decode(candidate.removeprefix(prefix), key, algorithms=["HS256"], issuer=ISSUER, options={"verify_exp": False, "verify_iat": False, "verify_nbf": False})
    except jwt.InvalidSignatureError:
        return {"error": "session token signature is invalid"}
    except (jwt.InvalidTokenError, TypeError, ValueError):
        return {"error": "session token is malformed"}
    if not isinstance(claims, dict) or claims.get("kind") != ("session_refresh" if refresh else "session"):
        return {"error": "session token is malformed"}
    if not all(isinstance(claims.get(name), str) and claims[name] for name in ("jti", "user_id", "client_id")):
        return {"error": "session token is malformed"}
    exp, iat = claims.get("exp"), claims.get("iat")
    if not isinstance(exp, int) or not isinstance(iat, int):
        return {"error": "session token is malformed"}
    if now >= exp:
        return {"error": "session token is expired"}
    return {"client_id": claims["client_id"], "exp": exp, "iat": iat, "jti": claims["jti"], "kind": claims["kind"], "user_id": claims["user_id"]}
