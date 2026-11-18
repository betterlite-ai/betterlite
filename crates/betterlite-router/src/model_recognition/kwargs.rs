use serde::Deserialize;
use serde::de::IgnoredAny;

// The call kwargs get_model_group_from_litellm_kwargs reads; keys it does not model are ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CallbackKwargs {
    #[serde(rename = "litellm_params")]
    pub params: Option<CallbackParams>,
    // get_metadata_variable_name_from_kwargs reads the outer litellm_metadata key's presence, null included.
    #[serde(rename = "litellm_metadata", with = "serde_with::rust::double_option")]
    pub outer_metadata: Option<Option<IgnoredAny>>,
}

// litellm_params' two metadata buckets, of which model_group is the one read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CallbackParams {
    pub metadata: Option<CallbackMetadata>,
    pub litellm_metadata: Option<CallbackMetadata>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CallbackMetadata {
    pub model_group: Option<String>,
}

// get_model_group_from_litellm_kwargs: the model_group a callback labels this request's deployment outcome with.
#[must_use]
pub fn model_group_from_kwargs(kwargs: &CallbackKwargs) -> Option<&str> {
    let params = kwargs.params.as_ref()?;
    let bucket = if kwargs.outer_metadata.is_some() {
        params.litellm_metadata.as_ref()
    } else {
        params.metadata.as_ref()
    };
    bucket?.model_group.as_deref()
}
