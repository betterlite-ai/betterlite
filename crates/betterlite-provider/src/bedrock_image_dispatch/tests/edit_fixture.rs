use std::time::{Duration, SystemTime};

use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage, EditPlan};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::bedrock_image_dispatch::call::{BedrockImageOperation, EditOperation};
use crate::bedrock_image_dispatch::test_support::{Catalog, NOW_SECONDS, Reply, instant};

use super::fixture::{Run, model_cost, run};

pub(crate) struct EditFixture {
    pub(crate) model: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) images: Vec<EditImage>,
    pub(crate) optional_params: ImageParams,
    pub(crate) extra_headers: Option<Vec<(String, String)>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) aimage_edit: bool,
    pub(crate) now: SystemTime,
}

impl EditFixture {
    pub(crate) fn new(
        model: Option<&str>,
        prompt: Option<&str>,
        images: Vec<EditImage>,
        optional_params: ImageParams,
    ) -> Self {
        let now = instant(NOW_SECONDS).expect("the fixture instant is after the Unix epoch");
        Self {
            model: model.map(str::to_owned),
            prompt: prompt.map(str::to_owned),
            images,
            optional_params,
            extra_headers: None,
            timeout: None,
            aimage_edit: false,
            now,
        }
    }

    pub(crate) fn operation<'fixture, 'cost, 'context>(
        &'fixture self,
        plan: EditPlan<'cost, 'context>,
    ) -> EditOperation<'fixture, 'cost, 'context> {
        EditOperation {
            model: self.model.as_deref(),
            prompt: self.prompt.as_deref(),
            images: &self.images,
            optional_params: self.optional_params.clone(),
            extra_headers: self.extra_headers.as_deref(),
            api_base: None,
            api_key: None,
            timeout: self.timeout,
            aimage_edit: self.aimage_edit,
            environment: EditEnvironment::default(),
            now: self.now,
            plan,
        }
    }
}

// The raw catalog entry the edit selector reads, plus the resolver the operation carries
pub(crate) struct EditSetup {
    model_cost: ParamMap,
    catalog: Catalog,
}

impl EditSetup {
    pub(crate) fn new() -> Self {
        Self {
            model_cost: model_cost(),
            catalog: Catalog::empty().expect("the empty catalog parses"),
        }
    }

    pub(crate) async fn run(&self, fixture: &EditFixture, reply: Reply) -> Run {
        let context = self.catalog.context();
        let plan = EditPlan {
            model_cost: &self.model_cost,
            resolution: &context,
        };
        run(BedrockImageOperation::Edit(fixture.operation(plan)), reply).await
    }
}
