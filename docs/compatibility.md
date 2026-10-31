# Compatibility with LiteLLM

betterlite behaves like LiteLLM 1.105.1 for every client, provider and config file that sends values of the types LiteLLM declares. It doesn't copy what Python tolerates beyond those types. This file says once, for every task, what betterlite does there instead.

The contract covers names (config keys, request and response fields, headers, env vars, routes), the values on the wire in responses and in requests to providers, an error's HTTP status, type, param and code, the defaults, and the types LiteLLM declares: its pydantic models, TypedDicts and type hints, and the OpenAI and provider API references it follows. Error messages are betterlite's own, as rule errors-match-litellm in .development/rules.yaml says.

It doesn't cover Python's runtime habits: truthiness (`x or default`, `if value:`), duck typing (a string where a bool is declared, a list where a dict is), repr and str text, json.dumps formatting, and object identity or changes made in place to the caller's dict. A setup that leans on one of them sends a value outside the declared type, so in betterlite it gets an error that names the field instead of a quiet difference.

What betterlite does instead:

- A key that is absent or null takes its default. An explicit value is used as given, 0, "" and [] included.
- A value the declared type can't hold is rejected where it enters, with an error that names the field: a load error for config, a 400 for a request.
- Env vars and `os.environ/` references are the one place where text turns into a bool or a number, and one shared parser does it for every field. A bool reads true, false, 1, 0, yes, no, on or off in any case, a number reads in decimal notation, and spaces around the value are trimmed. An empty env var counts as unset. Anything else is a load error that names the variable. LiteLLM spells this differently from place to place: [str_to_bool](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/main.py#L115-L134) reads only true and false, while the [Redis settings](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/_redis.py#L200) also take 1 and yes.
- Output is standard JSON with RFC 3339 timestamps. Key order, escaping, 300 against 300.0 and the digits of a timestamp's fraction are not part of the contract.
- Python object behavior isn't ported: no identity checks, no changes to a dict the caller passed in, no repr text.

Since this file covers these cases, a task doesn't list them in its deviation log and a review doesn't report them. Oracle cases send inputs of the declared type, because a case that feeds LiteLLM a value only Python accepts tests the accident. Every other difference a user can see still goes into the task's docs/deviations file, with a link to the LiteLLM line.
