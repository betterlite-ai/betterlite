use betterlite_config::sections::model_hub_credentials::Settings;
use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use secrecy::{ExposeSecret, SecretString};

use super::super::HubDispatchError;
use super::super::credentials::{
    Globals, HUGGINGFACE_CHAT_KEY, HUGGINGFACE_EMBEDDING_KEY, HUGGINGFACE_HUB_TOKEN,
    HUGGINGFACE_RERANK_BASE, HUGGINGFACE_RERANK_KEY, PREDIBASE_BASE, PREDIBASE_KEY,
    PREDIBASE_TENANT_ID, REPLICATE_KEY, Sources, Step, huggingface_env, resolve,
};
use super::support::{Env, empty_globals, environment, object};

fn resolved(
    chain: &[Step],
    call: Option<&str>,
    globals: &Globals<'_>,
    env: &Env,
    options: &mut Object,
) -> Option<String> {
    let sources = Sources { call, globals, env };
    let found = resolve(chain, &sources, options).unwrap();
    found.map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn the_settings_become_the_module_globals() {
    let settings = Settings {
        huggingface_key: Some(SecretString::from("hf")),
        replicate_key: Some(SecretString::from("r")),
        predibase_key: Some(SecretString::from("p")),
        predibase_tenant_id: Some(SecretString::from("t")),
    };
    let globals = Globals::from_settings(&settings);
    let shown = |value: Option<&SecretString>| value.map(ExposeSecret::expose_secret);
    assert_eq!(shown(globals.huggingface_key), Some("hf"));
    assert_eq!(shown(globals.replicate_key), Some("r"));
    assert_eq!(shown(globals.predibase_key), Some("p"));
    assert_eq!(shown(globals.predibase_tenant_id), Some("t"));
    assert!(globals.api_key.is_none());
    assert!(globals.api_base.is_none());
}

#[test]
fn huggingface_chat_key_follows_litellm_order() {
    let hf_global = SecretString::from("hf-global");
    let api = SecretString::from("api-global");
    let mut globals = empty_globals();
    globals.huggingface_key = Some(&hf_global);
    globals.api_key = Some(&api);
    let both = environment(&[("HF_TOKEN", "hf-token"), ("HUGGINGFACE_API_KEY", "hf-api")]);
    let chain = HUGGINGFACE_CHAT_KEY;
    let context = resolved(chain, Some("ctx"), &globals, &both, &mut Object::new());
    assert_eq!(context.as_deref(), Some("ctx"));
    let global = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(global.as_deref(), Some("hf-global"));
    globals.huggingface_key = None;
    let token = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(token.as_deref(), Some("hf-token"));
    let empty_token = environment(&[("HF_TOKEN", ""), ("HUGGINGFACE_API_KEY", "hf-api")]);
    let variable = resolved(chain, None, &globals, &empty_token, &mut Object::new());
    assert_eq!(variable.as_deref(), Some("hf-api"));
    let fallback = resolved(chain, None, &globals, &environment(&[]), &mut Object::new());
    assert_eq!(fallback.as_deref(), Some("api-global"));
    globals.api_key = None;
    assert!(resolved(chain, None, &globals, &environment(&[]), &mut Object::new()).is_none());
}

#[test]
fn huggingface_embedding_key_skips_hf_token() {
    let api = SecretString::from("api-global");
    let mut globals = empty_globals();
    globals.api_key = Some(&api);
    let chain = HUGGINGFACE_EMBEDDING_KEY;
    let both = environment(&[("HF_TOKEN", "hf-token"), ("HUGGINGFACE_API_KEY", "hf-api")]);
    let embedding = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(embedding.as_deref(), Some("hf-api"));
    let token_only = environment(&[("HF_TOKEN", "hf-token")]);
    let fallback = resolved(chain, None, &globals, &token_only, &mut Object::new());
    assert_eq!(fallback.as_deref(), Some("api-global"));
}

#[test]
fn huggingface_rerank_and_hub_keys_skip_hf_token() {
    let api = SecretString::from("api-global");
    let mut globals = empty_globals();
    globals.api_key = Some(&api);
    let mut options = Object::new();
    let both = environment(&[("HF_TOKEN", "hf-token"), ("HUGGINGFACE_API_KEY", "hf-api")]);
    let token_only = environment(&[("HF_TOKEN", "hf-token")]);
    let rerank = HUGGINGFACE_RERANK_KEY;
    let found = resolved(rerank, None, &globals, &both, &mut options);
    assert_eq!(found.as_deref(), Some("hf-api"));
    assert!(resolved(rerank, None, &globals, &token_only, &mut options).is_none());
    let hub = HUGGINGFACE_HUB_TOKEN;
    let token = resolved(hub, Some("ctx"), &globals, &both, &mut options);
    assert_eq!(token.as_deref(), Some("hf-api"));
    let empty = environment(&[("HUGGINGFACE_API_KEY", "")]);
    assert!(resolved(hub, None, &globals, &empty, &mut options).is_none());
}

#[test]
fn huggingface_env_keeps_a_variable_set_to_the_empty_string() {
    let present = environment(&[
        ("HF_API_BASE", ""),
        ("HUGGINGFACE_API_BASE", "http://huggingface"),
    ]);
    let expected = HuggingfaceEnv {
        hf_api_base: Some(String::new()),
        huggingface_api_base: Some(String::from("http://huggingface")),
    };
    assert_eq!(huggingface_env(&present), expected);
    assert_eq!(
        huggingface_env(&environment(&[])),
        HuggingfaceEnv::default()
    );
}

#[test]
fn huggingface_rerank_base_reads_the_call_then_the_two_variables() {
    let globals = empty_globals();
    let chain = HUGGINGFACE_RERANK_BASE;
    let both = environment(&[
        ("HF_API_BASE", "http://hf"),
        ("HUGGINGFACE_API_BASE", "http://huggingface"),
    ]);
    let call = resolved(
        chain,
        Some("http://call"),
        &globals,
        &both,
        &mut Object::new(),
    );
    assert_eq!(call.as_deref(), Some("http://call"));
    let first = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(first.as_deref(), Some("http://hf"));
    let empty_first = environment(&[
        ("HF_API_BASE", ""),
        ("HUGGINGFACE_API_BASE", "http://huggingface"),
    ]);
    let kept = resolved(chain, None, &globals, &empty_first, &mut Object::new());
    assert_eq!(kept.as_deref(), Some(""));
    let only_second = environment(&[("HUGGINGFACE_API_BASE", "http://huggingface")]);
    let second = resolved(chain, None, &globals, &only_second, &mut Object::new());
    assert_eq!(second.as_deref(), Some("http://huggingface"));
    assert!(resolved(chain, None, &globals, &environment(&[]), &mut Object::new()).is_none());
}

#[test]
fn replicate_key_follows_litellm_order() {
    let replicate_global = SecretString::from("replicate-global");
    let api = SecretString::from("api-global");
    let mut globals = empty_globals();
    globals.replicate_key = Some(&replicate_global);
    globals.api_key = Some(&api);
    let both = environment(&[
        ("REPLICATE_API_KEY", "replicate-env"),
        ("REPLICATE_API_TOKEN", "replicate-token"),
    ]);
    let chain = REPLICATE_KEY;
    let global = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(global.as_deref(), Some("replicate-global"));
    globals.replicate_key = None;
    let api_key = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(api_key.as_deref(), Some("api-global"));
    globals.api_key = None;
    let variable = resolved(chain, None, &globals, &both, &mut Object::new());
    assert_eq!(variable.as_deref(), Some("replicate-env"));
    let token_only = environment(&[("REPLICATE_API_TOKEN", "replicate-token")]);
    let token = resolved(chain, None, &globals, &token_only, &mut Object::new());
    assert_eq!(token.as_deref(), Some("replicate-token"));
    assert!(resolved(chain, None, &globals, &environment(&[]), &mut Object::new()).is_none());
}

#[test]
fn predibase_pops_tenant_id_before_predibase_tenant_id() {
    let tenant = SecretString::from("tenant-global");
    let mut globals = empty_globals();
    globals.predibase_tenant_id = Some(&tenant);
    let env = environment(&[("PREDIBASE_TENANT_ID", "tenant-env")]);
    let param = PromptJson::Text(String::from("tenant-param"));
    let mut both = object(&[
        ("tenant_id", PromptJson::Text(String::from("tenant-call"))),
        ("predibase_tenant_id", param.clone()),
    ]);
    let chain = PREDIBASE_TENANT_ID;
    let first = resolved(chain, None, &globals, &env, &mut both);
    assert_eq!(first.as_deref(), Some("tenant-call"));
    assert!(!both.contains_key("tenant_id"));
    assert!(both.contains_key("predibase_tenant_id"));
    let mut nul = object(&[
        ("tenant_id", PromptJson::Null),
        ("predibase_tenant_id", param),
    ]);
    let after_null = resolved(chain, None, &globals, &env, &mut nul);
    assert_eq!(after_null.as_deref(), Some("tenant-param"));
    assert!(nul.is_empty());
}

#[test]
fn predibase_tenant_id_falls_back_to_its_global_then_the_variable() {
    let tenant = SecretString::from("tenant-global");
    let mut globals = empty_globals();
    globals.predibase_tenant_id = Some(&tenant);
    let env = environment(&[("PREDIBASE_TENANT_ID", "tenant-env")]);
    let chain = PREDIBASE_TENANT_ID;
    let global = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(global.as_deref(), Some("tenant-global"));
    globals.predibase_tenant_id = None;
    let variable = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(variable.as_deref(), Some("tenant-env"));
    let empty = environment(&[("PREDIBASE_TENANT_ID", "")]);
    assert!(resolved(chain, None, &globals, &empty, &mut Object::new()).is_none());
}

#[test]
fn predibase_rejects_a_tenant_id_that_is_not_text() {
    let globals = empty_globals();
    let env = environment(&[]);
    let sources = Sources {
        call: None,
        globals: &globals,
        env: &env,
    };
    let mut options = object(&[("tenant_id", PromptJson::Bool(true))]);
    let error = resolve(PREDIBASE_TENANT_ID, &sources, &mut options).unwrap_err();
    let HubDispatchError::FieldType { field } = error else {
        panic!("a non-text tenant id is a field type error");
    };
    assert_eq!(field, "tenant_id");
}

#[test]
fn predibase_base_pops_api_base_then_base_url() {
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_BASE", "http://env")]);
    let mut both = object(&[
        ("api_base", PromptJson::Text(String::from("http://param"))),
        ("base_url", PromptJson::Text(String::from("http://other"))),
    ]);
    let chain = PREDIBASE_BASE;
    let call = resolved(chain, Some("http://call"), &globals, &env, &mut both);
    assert_eq!(call.as_deref(), Some("http://call"));
    assert_eq!(both.len(), 2);
    let param = resolved(chain, None, &globals, &env, &mut both);
    assert_eq!(param.as_deref(), Some("http://param"));
    assert!(!both.contains_key("api_base"));
    let other = resolved(chain, None, &globals, &env, &mut both);
    assert_eq!(other.as_deref(), Some("http://other"));
    assert!(both.is_empty());
}

#[test]
fn predibase_base_falls_back_to_the_global_then_the_variable() {
    let mut globals = empty_globals();
    globals.api_base = Some("http://litellm");
    let env = environment(&[("PREDIBASE_API_BASE", "http://env")]);
    let chain = PREDIBASE_BASE;
    let global = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(global.as_deref(), Some("http://litellm"));
    globals.api_base = None;
    let variable = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(variable.as_deref(), Some("http://env"));
    let empty = environment(&[("PREDIBASE_API_BASE", "")]);
    assert!(resolved(chain, None, &globals, &empty, &mut Object::new()).is_none());
}

#[test]
fn predibase_key_puts_litellm_api_key_before_the_predibase_key() {
    let api = SecretString::from("api-global");
    let predibase = SecretString::from("predibase-global");
    let mut globals = empty_globals();
    globals.api_key = Some(&api);
    globals.predibase_key = Some(&predibase);
    let env = environment(&[("PREDIBASE_API_KEY", "predibase-env")]);
    let chain = PREDIBASE_KEY;
    let api_key = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(api_key.as_deref(), Some("api-global"));
    globals.api_key = None;
    let global = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(global.as_deref(), Some("predibase-global"));
    globals.predibase_key = None;
    let variable = resolved(chain, None, &globals, &env, &mut Object::new());
    assert_eq!(variable.as_deref(), Some("predibase-env"));
    assert!(resolved(chain, None, &globals, &environment(&[]), &mut Object::new()).is_none());
}
