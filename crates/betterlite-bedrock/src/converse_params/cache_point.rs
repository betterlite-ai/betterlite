use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::catalog_flags::{
    accepts_cache_points, is_claude_4_5_on_bedrock,
};
use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::wire::transcode;
use crate::model_routes::{ParamMap, ParamValue};

const CACHE_CONTROL_FIELD: &str = "cache_control";
const CACHE_POINT_FIELD: &str = "cachePoint";
const DEFAULT_TYPE: &str = "default";
const TTL_FIELD: &str = "ttl";
const TYPE_FIELD: &str = "type";

// The system and content_block block types carry the same wire block, so one builder serves both.
pub fn get_cache_point_block(
    message_block: &ParamMap,
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<Option<Json>, ConverseParamsError> {
    let Some(cache_control) = message_block
        .get(CACHE_CONTROL_FIELD)
        .filter(|value| !matches!(value, ParamValue::Null))
    else {
        return Ok(None);
    };
    if !accepts_cache_points(model, model_cost)? {
        return Ok(None);
    }
    let cache_point = build_cache_point_block(Some(cache_control), model, model_cost)?;
    Ok(Some(Json::Object(IndexMap::from([(
        CACHE_POINT_FIELD.to_owned(),
        cache_point,
    )]))))
}

// type is always "default"; ttl survives only for Claude 4.5 on Bedrock, whatever the control's own type says.
pub fn build_cache_point_block(
    control: Option<&ParamValue>,
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<Json, ConverseParamsError> {
    let mut fields = IndexMap::from([(TYPE_FIELD.to_owned(), Json::Text(DEFAULT_TYPE.to_owned()))]);
    if let Some(ParamValue::Mapping(control_fields)) = control
        && let Some(ttl) = control_fields.get(TTL_FIELD)
        && matches!(ttl, ParamValue::Text(text) if text == "5m" || text == "1h")
        && let Some(name) = model
        && is_claude_4_5_on_bedrock(name, model_cost)?
    {
        fields.insert(TTL_FIELD.to_owned(), transcode(ttl)?);
    }
    Ok(Json::Object(fields))
}
