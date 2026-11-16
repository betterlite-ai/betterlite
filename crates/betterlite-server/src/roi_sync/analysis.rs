use std::error::Error as StdError;
use std::sync::{Arc, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::records::ROIReport;
use betterlite_roi::settings::ROISettings;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::roi_cache::store::{REPORT_PARAM, ReportRepository};
use crate::roi_estimator::CompletionCaller;
use crate::roi_github::client::GitHub;
use crate::roi_github::transport::Transport;
use crate::roi_spend::SpendReader;

use super::capabilities::CapabilityLookup;
use super::coordinator::SyncCoordinator;
use super::error::RunError;
use super::phases::{Execution, RunContext, execute};
use super::ports::{GitHubFactory, SyncClock};
use super::run::stop_monitor;
use super::status::StatusReporter;

pub(crate) struct Analyze<'run, T: Transport, Reader, Repository> {
    pub(crate) settings: &'run ROISettings,
    pub(crate) models: Option<&'run [EstimatorModel]>,
    pub(crate) complete: &'run CompletionCaller,
    pub(crate) spend_reader: &'run Reader,
    pub(crate) repository: &'run Repository,
    pub(crate) status: &'run StatusReporter,
    pub(crate) clock: &'run SyncClock,
    pub(crate) capabilities: &'run Arc<dyn CapabilityLookup>,
    pub(crate) coordinator: Option<&'run Arc<dyn SyncCoordinator>>,
    pub(crate) owner: &'run str,
    pub(crate) factory: &'run GitHubFactory<T>,
    pub(crate) transport: Option<T>,
}

impl<T, Reader, Repository> Analyze<'_, T, Reader, Repository>
where
    T: Transport,
    Reader: SpendReader + Sync,
    Repository: ReportRepository + Sync,
{
    pub(crate) async fn run(
        &mut self,
        client: &mut Option<GitHub<T>>,
        monitor: &mut JoinSet<()>,
        stop: &CancellationToken,
    ) -> Result<(), RunError<Repository::Error>> {
        let factory = self.factory;
        let opened = (factory)(self.settings, self.transport.take());
        let github = match opened {
            Ok(github) => github,
            Err(error) => return Err(RunError::Source(error)),
        };
        let outcome = self.run_phases(&github).await;
        *client = Some(github);
        let execution = outcome?;
        if let Some(handle) = client.take() {
            handle.close();
        }
        stop_monitor(monitor, stop).await;
        self.publish(&execution).await
    }

    async fn run_phases(
        &self,
        github: &GitHub<T>,
    ) -> Result<Execution, RunError<Repository::Error>> {
        let context = RunContext {
            settings: self.settings,
            models: self.models,
            complete: self.complete,
            github,
            status: self.status,
            clock: self.clock,
            capabilities: self.capabilities,
        };
        execute(&context, self.repository, self.spend_reader).await
    }

    async fn publish(&self, execution: &Execution) -> Result<(), RunError<Repository::Error>> {
        let completed = self.status.completed_status(execution.stage);
        match self.coordinator {
            Some(lease) => {
                if !lease
                    .finish(self.owner, &completed, Some(&execution.report))
                    .await
                {
                    return Err(RunError::Replaced);
                }
            }
            None => {
                let value = encode_report(&execution.report)?;
                self.repository.set_param(REPORT_PARAM, value).await?;
            }
        }
        let mut stored = self
            .status
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *stored = completed;
        Ok(())
    }
}

fn encode_report<Store>(report: &ROIReport) -> Result<JsonData, RunError<Store>>
where
    Store: StdError + 'static,
{
    let bytes = serde_json::to_vec(report)?;
    Ok(serde_json::from_slice(&bytes)?)
}
