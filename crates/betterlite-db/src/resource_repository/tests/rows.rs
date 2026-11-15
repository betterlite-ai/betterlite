use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use secrecy::SecretString;

use crate::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, LiteLLMObjectPermissionTable, PermissionList,
};
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::{FindMany, Key, KeyAction, KeyColumn};
use crate::resource_repository::budget::{BudgetRepository, BudgetTable};
use crate::resource_repository::config::{ConfigParam, ConfigRepository, ConfigUpdate};
use crate::resource_repository::credential::CredentialsRepository;
use crate::resource_repository::model::ModelRepository;
use crate::resource_repository::permission::ObjectPermissionRepository;
use crate::resource_repository::table::RowWrite;
use crate::resource_rows::credential::CredentialItem;
use crate::resource_rows::model::ProxyModelTable;

use super::support::{
    Case, cipher, defaults_actions, execute, field, insert_budget, insert_config,
    insert_credential, insert_model, insert_permission, keyed, seal, seal_with, text_row,
    text_rows, value, write, written,
};

fn model_repository(case: &Case, key: Option<SecretString>) -> ModelRepository<'_> {
    let publish = redis_config_change_publisher(None);
    ModelRepository::new(Some(case.client()), publish, key, cipher())
}

fn config_repository(case: &Case, use_writer: bool) -> ConfigRepository<'_> {
    ConfigRepository::new(Some(case.client()), use_writer)
}

fn credential_repository(case: &Case) -> CredentialsRepository<'_> {
    CredentialsRepository::new(Some(case.client()), redis_config_change_publisher(None))
}

fn budget_repository(case: &Case) -> BudgetRepository<'_> {
    BudgetRepository::new(Some(case.client()))
}

fn permission_repository(case: &Case, use_writer: bool) -> ObjectPermissionRepository<'_> {
    ObjectPermissionRepository::new(Some(case.client()), use_writer)
}

async fn permission_row(
    case: &Case,
    use_writer: bool,
    id: &str,
) -> Option<LiteLLMObjectPermissionTable> {
    let repository = permission_repository(case, use_writer);
    let table = repository.table().unwrap();
    table
        .as_ref()
        .find_unique::<LiteLLMObjectPermissionTable>(id)
        .await
        .unwrap()
}

fn pinned_model(id: &str, name: &str, params: &str, info: Option<&str>) -> ProxyModelTable {
    let info = info.unwrap_or("null");
    let text = format!(
        r#"{{"model_id":"{id}","model_name":"{name}","litellm_params":{params},"model_info":{info},"blocked":false,"created_at":"2026-01-02T03:04:05.123","created_by":"seed","updated_at":"2026-01-03T04:05:06.789","updated_by":"seed"}}"#
    );
    serde_json::from_str(&text).unwrap()
}

fn matching(models: &[ProxyModelTable], prefix: &str) -> Vec<ProxyModelTable> {
    models
        .iter()
        .filter(|model| model.fields.model_id.starts_with(prefix))
        .cloned()
        .collect()
}

pub(crate) async fn raw_model_table(case: &Case) {
    let repository = model_repository(case, None);
    let table = repository.table().unwrap();
    let stored = value(r#""{\"api_key\":\"sk-plain\"}""#);
    let created = table
        .create(write(&[
            ("model_id", value("\"raw-created\"")),
            ("model_name", value("\"raw-name\"")),
            ("litellm_params", stored.clone()),
            ("created_by", value("\"seed\"")),
            ("updated_by", value("\"seed\"")),
        ]))
        .await
        .unwrap();
    assert_eq!(field(&created, "model_id"), &value("\"raw-created\""));
    assert_eq!(field(&created, "litellm_params"), &stored);
    insert_model(
        case,
        "raw-inserted",
        "raw",
        r#"{"api_key":"sk-raw","n":1}"#,
        Some(r#"{"id":"raw"}"#),
    )
    .await;
    let rows = keyed(
        table.as_ref().find_many::<JsonData>().await.unwrap(),
        "model_id",
    );
    assert_eq!(
        field(&rows["raw-inserted"], "litellm_params"),
        &value(r#"{"api_key":"sk-raw","n":1}"#)
    );
    assert_eq!(
        field(&rows["raw-inserted"], "model_name"),
        &value("\"raw\"")
    );
}

pub(crate) async fn excluded_model_id(case: &Case) {
    insert_model(case, "excluded-keep", "keep", r#"{"api_key":"a"}"#, None).await;
    insert_model(case, "excluded-skip", "skip", r#"{"api_key":"b"}"#, None).await;
    let repository = model_repository(case, None);
    let models = repository.find_all_except("excluded-skip").await.unwrap();
    let ids: Vec<&str> = models
        .iter()
        .map(|model| model.fields.model_id.as_str())
        .collect();
    assert!(ids.contains(&"excluded-keep"));
    assert!(!ids.contains(&"excluded-skip"));
    let all = repository.find_all_except("excluded-absent").await.unwrap();
    let ids: Vec<&str> = all
        .iter()
        .map(|model| model.fields.model_id.as_str())
        .collect();
    assert!(ids.contains(&"excluded-keep"));
    assert!(ids.contains(&"excluded-skip"));
}

pub(crate) async fn model_json_columns(case: &Case) {
    let params =
        serde_json::to_string(&value(r#"{"api_key":"plain-string","list":[1,2]}"#)).unwrap();
    let info = serde_json::to_string(&value(r#"{"id":"decoded"}"#)).unwrap();
    insert_model(case, "json-columns", "json", &params, Some(&info)).await;
    let repository = model_repository(case, None);
    let models = matching(
        &repository.find_all_except("absent").await.unwrap(),
        "json-",
    );
    assert_eq!(
        models,
        vec![pinned_model(
            "json-columns",
            "json",
            r#"{"api_key":"plain-string","list":[1,2]}"#,
            Some(r#"{"id":"decoded"}"#)
        )]
    );
}

pub(crate) async fn model_nonstring_params(case: &Case) {
    let params = r#"{"s":"text","n":0.25,"i":7,"b":true,"z":null,"arr":[1,"x"],"obj":{"k":"v"}}"#;
    insert_model(case, "nonstring", "mixed", params, None).await;
    let models = matching(
        &model_repository(case, None)
            .find_all_except("absent")
            .await
            .unwrap(),
        "non",
    );
    assert_eq!(models.len(), 1);
    assert_eq!(
        models.first().unwrap(),
        &pinned_model("nonstring", "mixed", params, None)
    );
}

pub(crate) async fn model_decrypt_valid_plaintext_and_corrupt(case: &Case) {
    let (logs, _guard) = super::support::captured_logs();
    let params = format!(
        r#"{{"api_key":"{}","corrupt":"v2:gcm:bm90LXJlYWw","plain":"ordinary-text"}}"#,
        seal("decrypted-value")
    );
    insert_model(case, "sealed", "cipher", &params, None).await;
    let repository = model_repository(case, None);
    let models = matching(
        &repository.find_all_except("absent").await.unwrap(),
        "sealed",
    );
    let params = &models[0].fields.litellm_params.0;
    assert_eq!(params["api_key"], value("\"decrypted-value\""));
    assert_eq!(params["plain"], value("\"ordinary-text\""));
    assert_eq!(params["corrupt"], value("\"v2:gcm:bm90LXJlYWw\""));
    assert!(
        logs.text()
            .contains("Error decrypting value for key: corrupt")
    );
}

pub(crate) async fn constructor_key_not_read_key(case: &Case) {
    let caller_value = seal_with("caller-secret", "caller-key");
    let params = format!(
        r#"{{"api_key":"{}","caller":"{caller_value}"}}"#,
        seal("cipher-secret")
    );
    insert_model(case, "constructor-key", "keys", &params, None).await;
    let repository = model_repository(case, Some(SecretString::from("caller-key")));
    let models = matching(
        &repository.find_all_except("absent").await.unwrap(),
        "constructor",
    );
    let params = &models[0].fields.litellm_params.0;
    assert_eq!(params["api_key"], value("\"cipher-secret\""));
    assert_eq!(params["caller"], JsonData::String(caller_value));
}

pub(crate) async fn budget_id_column(case: &Case) {
    insert_budget(case, "budget-one").await;
    insert_budget(case, "budget-two").await;
    let repository = budget_repository(case);
    let expected: LiteLLMBudgetTable = serde_json::from_str(
        r#"{"budget_id":"budget-one","soft_budget":2.5,"max_budget":10,"max_parallel_requests":3,"tpm_limit":40,"rpm_limit":50,"model_max_budget":{"gpt-4o":0.5},"budget_duration":"1d"}"#,
    )
    .unwrap();
    assert_eq!(
        repository.find_by_id("budget-one").await.unwrap(),
        Some(expected)
    );
    assert!(repository.find_by_id("1d").await.unwrap().is_none());
    assert!(
        repository
            .find_by_id("budget-missing")
            .await
            .unwrap()
            .is_none()
    );
    budget_b700_actions(case).await;
}

async fn budget_b700_actions(case: &Case) {
    let client = case.reader.session().await.unwrap();
    let b700 = Repository {
        client: Some(&client),
        table: BudgetTable,
    };
    assert_eq!(b700.find_many(FindMany::default()).await.unwrap().len(), 2);
    assert_eq!(b700.count(None).await.unwrap(), 2);
    let one = b700
        .find_many_in(KeyColumn::Id, vec!["budget-one".to_owned()])
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    let no_data = ();
    let update = KeyAction::Update(&no_data);
    let key = Key {
        column: KeyColumn::Id,
        value: &"budget-one",
    };
    assert!(b700.by_key(update, key).await.unwrap().is_some());
    let key = Key {
        column: KeyColumn::Id,
        value: &"budget-two",
    };
    assert!(b700.by_key(KeyAction::Delete, key).await.unwrap().is_some());
    let key = Key {
        column: KeyColumn::Id,
        value: &"budget-two",
    };
    assert!(b700.by_key(KeyAction::Find, key).await.unwrap().is_none());
}

pub(crate) async fn permission_writer_choice(case: &Case) {
    insert_permission(&case.reader, "perm-reader", "reader").await;
    insert_permission(&case.writer, "perm-writer", "writer").await;
    let reader = permission_row(case, false, "perm-reader").await.unwrap();
    assert_eq!(
        reader.mcp_servers,
        PermissionList(Some(vec!["reader".to_owned()]))
    );
    assert!(permission_row(case, false, "perm-writer").await.is_none());
    let writer = permission_row(case, true, "perm-writer").await.unwrap();
    assert_eq!(
        writer.mcp_servers,
        PermissionList(Some(vec!["writer".to_owned()]))
    );
    assert!(permission_row(case, true, "perm-reader").await.is_none());
    permission_writes_always_reach_writer(case).await;
}

async fn permission_writes_always_reach_writer(case: &Case) {
    let table = permission_repository(case, false).table().unwrap();
    table
        .upsert("perm-via-reader".to_owned(), RowWrite::default())
        .await
        .unwrap();
    let sql = "SELECT object_permission_id FROM \"LiteLLM_ObjectPermissionTable\" \
               WHERE object_permission_id = 'perm-via-reader'";
    assert_eq!(text_row(&case.writer, sql, &[]).await, "perm-via-reader");
    assert!(text_rows(&case.reader, sql, &[]).await.is_empty());
}

pub(crate) async fn config_writer_choice(case: &Case) {
    insert_config(&case.reader, "config-shared", "\"reader\"").await;
    insert_config(&case.writer, "config-shared", "\"writer\"").await;
    insert_config(&case.writer, "config-writer-only", "7").await;
    let reading = config_repository(case, false);
    let shared = reading.get_param("config-shared").await.unwrap().unwrap();
    assert_eq!(shared.param_value, JsonData::String("reader".to_owned()));
    assert!(
        reading
            .get_param("config-writer-only")
            .await
            .unwrap()
            .is_none()
    );
    let writing = config_repository(case, true);
    let shared = writing.get_param("config-shared").await.unwrap().unwrap();
    assert_eq!(shared.param_value, JsonData::String("writer".to_owned()));
    let only = writing
        .get_param("config-writer-only")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        only.param_value,
        JsonData::Number(serde_json::Number::from(7_i32))
    );
    config_write_through_reader(case).await;
}

async fn config_write_through_reader(case: &Case) {
    let reading = config_repository(case, false);
    reading
        .set_param("config-through-reader", JsonData::Null)
        .await
        .unwrap();
    assert!(
        reading
            .get_param("config-through-reader")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        config_repository(case, true)
            .get_param("config-through-reader")
            .await
            .unwrap()
            .is_some()
    );
}

pub(crate) async fn config_missing(case: &Case) {
    let repository = config_repository(case, false);
    assert!(
        repository
            .get_param("config-absent")
            .await
            .unwrap()
            .is_none()
    );
    execute(
        &case.writer,
        "INSERT INTO \"LiteLLM_Config\" (param_name, param_value) VALUES ('config-null', NULL)",
    )
    .await;
    let null = repository.get_param("config-null").await.unwrap().unwrap();
    assert_eq!(
        null,
        ConfigParam {
            param_name: "config-null".to_owned(),
            param_value: JsonData::Null
        }
    );
    execute(
        &case.writer,
        "INSERT INTO \"LiteLLM_Config\" (param_name, param_value) VALUES ('config-object', '{\"k\":1}'::jsonb)",
    )
    .await;
    let object = repository
        .get_param("config-object")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(object.param_value, value(r#"{"k":1}"#));
}

pub(crate) async fn config_create_update_fields(case: &Case) {
    execute(
        &case.writer,
        "INSERT INTO \"LiteLLM_Config\" (param_name, param_value, last_run_at, reload_revision) \
         VALUES ('config-tracked', to_jsonb('\"old\"'::text), '2026-02-03 04:05:06', 7)",
    )
    .await;
    let repository = config_repository(case, false);
    repository
        .set_param("config-tracked", JsonData::String("new".to_owned()))
        .await
        .unwrap();
    repository
        .set_param("config-fresh", JsonData::Bool(true))
        .await
        .unwrap();
    let tracked = text_row(
        &case.writer,
        "SELECT jsonb_typeof(param_value) || ':' || (param_value #>> '{}') || ':' || \
         COALESCE(last_run_at::text, 'null') || ':' || reload_revision::text \
         FROM \"LiteLLM_Config\" WHERE param_name = 'config-tracked'",
        &[],
    )
    .await;
    assert_eq!(tracked, "string:new:2026-02-03 04:05:06:7");
    let fresh = text_row(
        &case.writer,
        "SELECT jsonb_typeof(param_value) || ':' || (param_value #>> '{}') || ':' || \
         (last_run_at IS NULL)::text || ':' || reload_revision::text \
         FROM \"LiteLLM_Config\" WHERE param_name = 'config-fresh'",
        &[],
    )
    .await;
    assert_eq!(fresh, "string:true:true:0");
}

pub(crate) async fn config_original_return_value(case: &Case) {
    let repository = config_repository(case, false);
    let object = value(r#"{"a":[1,2],"b":null}"#);
    let returned = repository
        .set_param("config-return-object", object.clone())
        .await
        .unwrap();
    assert_eq!(
        returned,
        ConfigParam {
            param_name: "config-return-object".to_owned(),
            param_value: object
        }
    );
    let number = repository
        .set_param(
            "config-return-number",
            JsonData::Number(serde_json::Number::from(12_i32)),
        )
        .await
        .unwrap();
    assert_eq!(
        number.param_value,
        JsonData::Number(serde_json::Number::from(12_i32))
    );
    let null = repository
        .set_param("config-return-null", JsonData::Null)
        .await
        .unwrap();
    assert_eq!(null.param_value, JsonData::Null);
}

pub(crate) async fn config_verbatim_string(case: &Case) {
    let repository = config_repository(case, false);
    let bare = JsonData::String("abc".to_owned());
    let returned = repository.set_param("config-bare", bare).await.unwrap();
    assert_eq!(returned.param_value, JsonData::String("abc".to_owned()));
    let quoted = JsonData::String("\"hello\"".to_owned());
    let returned = repository.set_param("config-quoted", quoted).await.unwrap();
    assert_eq!(
        returned.param_value,
        JsonData::String("\"hello\"".to_owned())
    );
    let read = repository
        .get_param("config-quoted")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.param_value, JsonData::String("hello".to_owned()));
    config_number_stored_as_text(case).await;
}

async fn config_number_stored_as_text(case: &Case) {
    config_repository(case, false)
        .set_param(
            "config-number",
            JsonData::Number(serde_json::Number::from(12_i32)),
        )
        .await
        .unwrap();
    let stored = text_row(
        &case.writer,
        "SELECT jsonb_typeof(param_value) || ':' || (param_value #>> '{}') \
         FROM \"LiteLLM_Config\" WHERE param_name = 'config-number'",
        &[],
    )
    .await;
    assert_eq!(stored, "string:12");
}

pub(crate) async fn credential_verbatim_writes(case: &Case) {
    let repository = credential_repository(case);
    let created = repository
        .create(written(
            r#"{"credential_id":"cred-keep-id","credential_name":"cred-keep","credential_values":{"api_key":"v2:gcm:verbatim","custom":"x"},"credential_info":{"note":"n"},"created_by":"seed","updated_by":"seed"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        field(&created, "credential_values"),
        &value(r#"{"api_key":"v2:gcm:verbatim","custom":"x"}"#)
    );
    credential_partial_update(&repository, case).await;
}

async fn credential_partial_update(repository: &CredentialsRepository<'_>, case: &Case) {
    let updated = repository
        .update_by_name(
            "cred-keep",
            written(
                r#"{"credential_info":{"note":"changed"},"updated_at":"2026-04-05T06:07:08.9"}"#,
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        field(&updated, "credential_info"),
        &value(r#"{"note":"changed"}"#)
    );
    assert_eq!(field(&updated, "credential_id"), &value("\"cred-keep-id\""));
    assert_eq!(
        field(&updated, "credential_values"),
        &value(r#"{"api_key":"v2:gcm:verbatim","custom":"x"}"#)
    );
    let stored = text_row(
        &case.writer,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS.MS') \
         FROM \"LiteLLM_CredentialsTable\" WHERE credential_name = 'cred-keep'",
        &[],
    )
    .await;
    assert_eq!(stored, "2026-04-05 06:07:08.900");
    let missing = repository
        .update_by_name("cred-absent", written(r#"{"credential_values":{"a":"b"}}"#))
        .await
        .unwrap();
    assert!(missing.is_none());
}

pub(crate) async fn credential_upsert_preserves_id(case: &Case) {
    let repository = credential_repository(case);
    repository
        .create(written(
            r#"{"credential_id":"cred-upsert-id","credential_name":"cred-upsert","credential_values":{"api_key":"before"},"created_by":"seed","updated_by":"seed"}"#,
        ))
        .await
        .unwrap();
    let table = repository.table().unwrap();
    let row = table
        .upsert(
            "cred-upsert".to_owned(),
            written(r#"{"credential_values":{"api_key":"after"}}"#),
        )
        .await
        .unwrap();
    assert_eq!(field(&row, "credential_id"), &value("\"cred-upsert-id\""));
    assert_eq!(
        field(&row, "credential_values"),
        &value(r#"{"api_key":"after"}"#)
    );
    let fresh = table
        .upsert(
            "cred-fresh-name".to_owned(),
            written(r#"{"credential_values":{"a":"b"},"created_by":"seed","updated_by":"seed"}"#),
        )
        .await
        .unwrap();
    let id = field(&fresh, "credential_id").clone();
    assert!(matches!(id, JsonData::String(ref text) if !text.is_empty()));
}

pub(crate) async fn credential_raw_list(case: &Case) {
    let repository = credential_repository(case);
    create_listed_credential(&repository, "list-a", "a").await;
    create_listed_credential(&repository, "list-b", "b").await;
    let rows = keyed(repository.find_all().await.unwrap(), "credential_name");
    assert_eq!(
        field(&rows["list-a"], "credential_values"),
        &value(r#"{"api_key":"a"}"#)
    );
    assert_eq!(
        field(&rows["list-b"], "credential_values"),
        &value(r#"{"api_key":"b"}"#)
    );
    assert_eq!(field(&rows["list-a"], "credential_info"), &JsonData::Null);
}

async fn create_listed_credential(repository: &CredentialsRepository<'_>, name: &str, key: &str) {
    repository
        .create(written(&format!(
            r#"{{"credential_name":"{name}","credential_values":{{"api_key":"{key}"}},"created_by":"seed","updated_by":"seed"}}"#
        )))
        .await
        .unwrap();
}

pub(crate) async fn credential_named_lookup(case: &Case) {
    insert_credential(
        &case.writer,
        "cred-lookup-id",
        "cred-lookup-name",
        r#"{"api_key":"sk-raw"}"#,
        None,
    )
    .await;
    let repository = credential_repository(case);
    let found = repository
        .find_by_name("cred-lookup-name")
        .await
        .unwrap()
        .unwrap();
    let expected: CredentialItem = serde_json::from_str(
        r#"{"credential_name":"cred-lookup-name","credential_info":{},"credential_values":{"api_key":"sk-raw"}}"#,
    )
    .unwrap();
    assert_eq!(found, expected);
    assert!(
        repository
            .find_by_name("cred-lookup-id")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repository
            .find_by_name("cred-absent")
            .await
            .unwrap()
            .is_none()
    );
    credential_delete_by_name(&repository).await;
}

async fn credential_delete_by_name(repository: &CredentialsRepository<'_>) {
    let deleted = repository
        .delete_by_name("cred-lookup-name")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        field(&deleted, "credential_id"),
        &value("\"cred-lookup-id\"")
    );
    assert!(
        repository
            .delete_by_name("cred-lookup-name")
            .await
            .unwrap()
            .is_none()
    );
}

pub(crate) async fn falsey_credential_maps(case: &Case) {
    let falsey = [
        ("cred-false-id", "cred-false", Some("false")),
        ("cred-zero-id", "cred-zero", Some("0")),
        ("cred-empty-string-id", "cred-empty-string", Some("\"\"")),
        ("cred-empty-array-id", "cred-empty-array", Some("[]")),
        ("cred-null-info-id", "cred-null-info", None),
    ];
    for (id, name, info) in falsey {
        insert_credential(&case.writer, id, name, r#"{"api_key":"k"}"#, info).await;
    }
    insert_credential(
        &case.writer,
        "cred-empty-values-id",
        "cred-empty-values",
        "{}",
        Some(r#"{"flag":true}"#),
    )
    .await;
    let repository = credential_repository(case);
    for (_, name, _) in falsey {
        let row = repository.find_by_name(name).await.unwrap().unwrap();
        assert!(row.fields.base.credential_info.0.is_empty());
        assert_eq!(row.fields.credential_values.0.len(), 1);
    }
    let empty = repository
        .find_by_name("cred-empty-values")
        .await
        .unwrap()
        .unwrap();
    assert!(empty.fields.credential_values.0.is_empty());
    assert_eq!(empty.fields.base.credential_info.0.len(), 1);
}

pub(crate) async fn empty_document_create(case: &Case) {
    let table = defaults_actions(case);
    let created = table.create(RowWrite::default()).await.unwrap();
    let JsonData::String(id) = field(&created, "id").clone() else {
        panic!("a generated default id");
    };
    assert!(id.starts_with("id-"));
    assert_eq!(field(&created, "payload"), &JsonData::Null);
    let count = table
        .create_many(vec![RowWrite::default(), RowWrite::default()])
        .await
        .unwrap();
    assert_eq!(count, 2);
    let rows = table.as_ref().find_many::<JsonData>().await.unwrap();
    assert_eq!(rows.len(), 3);
}

pub(crate) async fn empty_document_write(case: &Case) {
    let configs = config_repository(case, false);
    configs
        .set_param("empty-update", JsonData::String("value".to_owned()))
        .await
        .unwrap();
    let table = configs.table().unwrap();
    let changed = table
        .update(ConfigUpdate::default(), "empty-update".to_owned())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(field(&changed, "param_value"), &value("\"value\""));
    let permissions = permission_repository(case, false);
    let table = permissions.table().unwrap();
    let upserted = table
        .upsert("perm-empty-upsert".to_owned(), RowWrite::default())
        .await
        .unwrap();
    assert_eq!(
        field(&upserted, "object_permission_id"),
        &value("\"perm-empty-upsert\"")
    );
    let updated = table
        .update(RowWrite::default(), "perm-empty-upsert".to_owned())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        field(&updated, "object_permission_id"),
        &value("\"perm-empty-upsert\"")
    );
}

pub(crate) async fn partial_update_preserves_columns(case: &Case) {
    insert_model(
        case,
        "partial-update",
        "before",
        r#"{"api_key":"a","n":1}"#,
        Some(r#"{"id":"keep"}"#),
    )
    .await;
    let repository = model_repository(case, None);
    let table = repository.table().unwrap();
    let updated = table
        .update(
            write(&[("model_name", value("\"after\""))]),
            "partial-update".to_owned(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(field(&updated, "model_name"), &value("\"after\""));
    assert_eq!(
        field(&updated, "litellm_params"),
        &value(r#"{"api_key":"a","n":1}"#)
    );
    assert_eq!(field(&updated, "model_info"), &value(r#"{"id":"keep"}"#));
    assert_eq!(field(&updated, "created_by"), &value("\"seed\""));
    assert_ne!(
        field(&updated, "updated_at"),
        &value("\"2026-01-03T04:05:06.789\"")
    );
    assert!(
        table
            .update(RowWrite::default(), "partial-absent".to_owned())
            .await
            .unwrap()
            .is_none()
    );
}

pub(crate) async fn bulk_writes(case: &Case) {
    let configs = config_repository(case, false);
    configs
        .set_param("bulk-config", JsonData::String("\"one\"".to_owned()))
        .await
        .unwrap();
    let table = configs.table().unwrap();
    let update = ConfigUpdate {
        param_value: Some(JsonData::Number(serde_json::Number::from(3_i32))),
    };
    assert_eq!(
        table
            .update_many(update, "bulk-config".to_owned())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        table
            .update_many(ConfigUpdate::default(), "bulk-absent".to_owned())
            .await
            .unwrap(),
        0
    );
    let stored = configs.get_param("bulk-config").await.unwrap().unwrap();
    assert_eq!(
        stored.param_value,
        JsonData::Number(serde_json::Number::from(3_i32))
    );
    credential_bulk(case).await;
}

async fn credential_bulk(case: &Case) {
    let table = credential_repository(case).table().unwrap();
    let created = table
        .create_many(vec![
            written(r#"{"credential_name":"bulk-a","credential_values":{"api_key":"a"},"created_by":"seed","updated_by":"seed"}"#),
            written(r#"{"credential_name":"bulk-b","credential_values":{"api_key":"b"},"created_by":"seed","updated_by":"seed"}"#),
        ])
        .await
        .unwrap();
    assert_eq!(created, 2);
    let names = text_rows(
        &case.writer,
        "SELECT credential_name FROM \"LiteLLM_CredentialsTable\" \
         WHERE credential_name LIKE 'bulk-%' ORDER BY credential_name",
        &[],
    )
    .await;
    assert_eq!(names, vec!["bulk-a".to_owned(), "bulk-b".to_owned()]);
    assert_eq!(
        table
            .as_ref()
            .delete_many("bulk-a".to_owned())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        table
            .as_ref()
            .delete_many("bulk-a".to_owned())
            .await
            .unwrap(),
        0
    );
}
