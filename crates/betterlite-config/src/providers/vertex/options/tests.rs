#![cfg(test)]

mod gemini_option_settings {
    use betterlite_models::decisions::Json;
    use serde::Deserialize;

    use crate::loader::{ConfigError, parse_view};

    // the generation values LiteLLM constructs a deployment's config with; every one starts unset
    #[derive(Debug, Default, Deserialize, PartialEq)]
    pub(crate) struct GenerationDefaults {
        pub(crate) temperature: Option<f64>,
        pub(crate) max_output_tokens: Option<i64>,
        pub(crate) top_p: Option<f64>,
        pub(crate) top_k: Option<i64>,
        pub(crate) response_mime_type: Option<String>,
        pub(crate) candidate_count: Option<i64>,
        pub(crate) stop_sequences: Option<Vec<String>>,
        pub(crate) frequency_penalty: Option<f64>,
        pub(crate) presence_penalty: Option<f64>,
        pub(crate) seed: Option<i64>,
    }

    #[derive(Debug, Default, PartialEq)]
    pub(crate) struct Settings {
        pub(crate) deployments: Vec<GenerationDefaults>,
        pub(crate) safety_settings: Option<Json>,
    }

    #[derive(Debug, Default, Deserialize)]
    struct Raw {
        #[serde(default)]
        model_list: Vec<RawDeployment>,
        #[serde(default)]
        litellm_settings: RawLiteLlm,
    }

    #[derive(Debug, Default, Deserialize)]
    struct RawDeployment {
        #[serde(default)]
        litellm_params: GenerationDefaults,
    }

    #[derive(Debug, Default, Deserialize)]
    struct RawLiteLlm {
        vertex_ai_safety_settings: Option<Json>,
    }

    // The section registry in sections.rs has no entry for this key yet, so the loader's own view parses it here until it does
    pub(crate) fn load(source: &str) -> Result<Settings, ConfigError> {
        let (raw, _unknown) = parse_view::<Raw>(source)?;
        Ok(Settings {
            deployments: raw
                .model_list
                .into_iter()
                .map(|deployment| deployment.litellm_params)
                .collect(),
            safety_settings: raw.litellm_settings.vertex_ai_safety_settings,
        })
    }

    use betterlite_models::decisions::Json;

    use super::{GenerationDefaults, Settings, load};

    const CONFIGURED: &str = "\
    litellm_settings:
      vertex_ai_safety_settings:
        category: HARM_CATEGORY_HATE_SPEECH
        threshold: BLOCK_LOW_AND_ABOVE
    model_list:
      - litellm_params:
          model: vertex_ai/gemini-2.5-pro
          temperature: 0
          max_output_tokens: 256
          top_p: 0.95
          top_k: 40
          response_mime_type: application/json
          candidate_count: 2
          stop_sequences: [END]
          frequency_penalty: 0.1
          presence_penalty: 0.2
          seed: 7
      - litellm_params:
          model: vertex_ai/gemini-3-pro-preview
    ";

    fn configured() -> GenerationDefaults {
        GenerationDefaults {
            temperature: Some(0.0),
            max_output_tokens: Some(256),
            top_p: Some(0.95),
            top_k: Some(40),
            response_mime_type: Some("application/json".to_owned()),
            candidate_count: Some(2),
            stop_sequences: Some(vec!["END".to_owned()]),
            frequency_penalty: Some(0.1),
            presence_penalty: Some(0.2),
            seed: Some(7),
        }
    }

    #[test]
    fn loads_generation_defaults_and_safety_settings() {
        let settings = load(CONFIGURED).expect("the loader's view parses the keys");

        assert_eq!(
            settings,
            Settings {
                deployments: vec![configured(), GenerationDefaults::default()],
                safety_settings: Some(Json::Object(
                    [
                        (
                            "category".to_owned(),
                            Json::Text("HARM_CATEGORY_HATE_SPEECH".to_owned())
                        ),
                        (
                            "threshold".to_owned(),
                            Json::Text("BLOCK_LOW_AND_ABOVE".to_owned())
                        ),
                    ]
                    .into_iter()
                    .collect()
                )),
            }
        );
    }

    #[test]
    fn an_unset_safety_override_stays_absent() {
        let settings = load("model_list: []\n").expect("a config without the module key loads");

        assert_eq!(settings.safety_settings, None);
        assert!(settings.deployments.is_empty());
    }

    #[test]
    fn a_null_safety_override_is_absent() {
        let settings = load("litellm_settings:\n  vertex_ai_safety_settings: null\n")
            .expect("a null key takes the default");

        assert_eq!(settings.safety_settings, None);
    }

    #[test]
    fn a_non_numeric_generation_value_names_itself() {
        let error = load("model_list:\n  - litellm_params:\n      temperature: warm\n")
            .expect_err("a string is not a temperature");

        assert!(error.to_string().contains("temperature"), "{error}");
    }
}
