use std::collections::BTreeMap;
use std::fmt;
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabelDumpValue {
    Null,
    Boolean(bool),
    Integer(i64),
    String(String),
    Strings(Vec<String>),
    Metadata(BTreeMap<String, String>),
}

impl LabelDumpValue {
    const fn strings(&self) -> Option<&Vec<String>> {
        if let Self::Strings(values) = self {
            Some(values)
        } else {
            None
        }
    }

    const fn metadata(&self) -> Option<&BTreeMap<String, String>> {
        if let Self::Metadata(values) = self {
            Some(values)
        } else {
            None
        }
    }
}
#[derive(Clone, Default, Eq, PartialEq)]
pub struct UserApiKeyLabelValues {
    pub end_user: Option<String>,
    pub user: Option<String>,
    pub user_email: Option<String>,
    pub user_alias: Option<String>,
    pub hashed_api_key: Option<String>,
    pub api_key_alias: Option<String>,
    pub team: Option<String>,
    pub team_alias: Option<String>,
    pub model_group: Option<String>,
    pub requested_model: Option<String>,
    pub model: Option<String>,
    pub litellm_model_name: Option<String>,
    pub tags: Vec<String>,
    pub custom_metadata_labels: BTreeMap<String, String>,
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub api_provider: Option<String>,
    pub exception_status: Option<String>,
    pub exception_class: Option<String>,
    pub rate_limit_category: Option<String>,
    pub rate_limit_type: Option<String>,
    pub status_code: Option<String>,
    pub fallback_model: Option<String>,
    pub route: Option<String>,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub stream: Option<String>,
    pub org_id: Option<String>,
    pub org_alias: Option<String>,
    pub mcp_tool_name: Option<String>,
    pub mcp_server_name: Option<String>,
    pub service_tier: Option<String>,
    pub input_sequence_length: Option<String>,
}
impl UserApiKeyLabelValues {
    fn set_tags(&mut self, value: &LabelDumpValue) {
        if let Some(tags) = value.strings() {
            self.tags.clone_from(tags);
        }
    }

    fn set_metadata(&mut self, value: &LabelDumpValue) {
        if let Some(metadata) = value.metadata() {
            self.custom_metadata_labels.clone_from(metadata);
        }
    }

    #[must_use]
    pub fn from_entries(values: &[(String, LabelDumpValue)]) -> Self {
        let mut result = Self::default();
        for (name, value) in values {
            let canonical = if name == "api_key_hash" {
                "hashed_api_key"
            } else {
                name.as_str()
            };
            if canonical == "tags" {
                result.set_tags(value);
                continue;
            }
            if canonical == "custom_metadata_labels" {
                result.set_metadata(value);
                continue;
            }
            result.set_scalar(canonical, value);
        }
        result
    }
    fn set_scalar(&mut self, name: &str, value: &LabelDumpValue) {
        let Some(target) = self.string_field_mut(name) else {
            return;
        };
        match value {
            LabelDumpValue::Null => *target = None,
            LabelDumpValue::String(text) => *target = Some(text.clone()),
            LabelDumpValue::Boolean(flag) if name == "stream" => {
                *target = Some(if *flag { "True" } else { "False" }.to_owned());
            }
            LabelDumpValue::Integer(number) if name == "stream" => {
                *target = Some(number.to_string());
            }
            LabelDumpValue::Boolean(_)
            | LabelDumpValue::Integer(_)
            | LabelDumpValue::Strings(_)
            | LabelDumpValue::Metadata(_) => {}
        }
    }
    #[must_use]
    pub fn model_dump(&self) -> BTreeMap<String, LabelDumpValue> {
        let mut dump = BTreeMap::new();
        self.write_strings(&mut dump);
        dump.insert(
            "tags".to_owned(),
            LabelDumpValue::Strings(self.tags.clone()),
        );
        dump.insert(
            "custom_metadata_labels".to_owned(),
            LabelDumpValue::Metadata(self.custom_metadata_labels.clone()),
        );
        dump
    }
    fn write_strings(&self, dump: &mut BTreeMap<String, LabelDumpValue>) {
        for (name, value) in [
            ("end_user", self.end_user.clone()),
            ("user", self.user.clone()),
            ("user_email", self.user_email.clone()),
            ("user_alias", self.user_alias.clone()),
            ("hashed_api_key", self.hashed_api_key.clone()),
            ("api_key_alias", self.api_key_alias.clone()),
            ("team", self.team.clone()),
            ("team_alias", self.team_alias.clone()),
            ("model_group", self.model_group.clone()),
            ("requested_model", self.requested_model.clone()),
            ("model", self.model.clone()),
            ("litellm_model_name", self.litellm_model_name.clone()),
            ("model_id", self.model_id.clone()),
            ("api_base", self.api_base.clone()),
            ("api_provider", self.api_provider.clone()),
            ("exception_status", self.exception_status.clone()),
            ("exception_class", self.exception_class.clone()),
            ("rate_limit_category", self.rate_limit_category.clone()),
            ("rate_limit_type", self.rate_limit_type.clone()),
            ("status_code", self.status_code.clone()),
            ("fallback_model", self.fallback_model.clone()),
            ("route", self.route.clone()),
            ("client_ip", self.client_ip.clone()),
            ("user_agent", self.user_agent.clone()),
            ("stream", self.stream.clone()),
            ("org_id", self.org_id.clone()),
            ("org_alias", self.org_alias.clone()),
            ("mcp_tool_name", self.mcp_tool_name.clone()),
            ("mcp_server_name", self.mcp_server_name.clone()),
            ("service_tier", self.service_tier.clone()),
            ("input_sequence_length", self.input_sequence_length.clone()),
        ] {
            dump.insert(
                name.to_owned(),
                value.map_or(LabelDumpValue::Null, LabelDumpValue::String),
            );
        }
    }
    fn string_field_mut(&mut self, name: &str) -> Option<&mut Option<String>> {
        match name {
            "end_user" => Some(&mut self.end_user),
            "user" => Some(&mut self.user),
            "user_email" => Some(&mut self.user_email),
            "user_alias" => Some(&mut self.user_alias),
            "hashed_api_key" => Some(&mut self.hashed_api_key),
            "api_key_alias" => Some(&mut self.api_key_alias),
            "team" => Some(&mut self.team),
            "team_alias" => Some(&mut self.team_alias),
            "model_group" => Some(&mut self.model_group),
            "requested_model" => Some(&mut self.requested_model),
            "model" => Some(&mut self.model),
            "litellm_model_name" => Some(&mut self.litellm_model_name),
            "model_id" => Some(&mut self.model_id),
            "api_base" => Some(&mut self.api_base),
            "api_provider" => Some(&mut self.api_provider),
            "exception_status" => Some(&mut self.exception_status),
            "exception_class" => Some(&mut self.exception_class),
            "rate_limit_category" => Some(&mut self.rate_limit_category),
            "rate_limit_type" => Some(&mut self.rate_limit_type),
            "status_code" => Some(&mut self.status_code),
            "fallback_model" => Some(&mut self.fallback_model),
            "route" => Some(&mut self.route),
            "client_ip" => Some(&mut self.client_ip),
            "user_agent" => Some(&mut self.user_agent),
            "stream" => Some(&mut self.stream),
            "org_id" => Some(&mut self.org_id),
            "org_alias" => Some(&mut self.org_alias),
            "mcp_tool_name" => Some(&mut self.mcp_tool_name),
            "mcp_server_name" => Some(&mut self.mcp_server_name),
            "service_tier" => Some(&mut self.service_tier),
            "input_sequence_length" => Some(&mut self.input_sequence_length),
            _ => None,
        }
    }
}

impl fmt::Debug for UserApiKeyLabelValues {
    fn fmt(&self, _formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}
