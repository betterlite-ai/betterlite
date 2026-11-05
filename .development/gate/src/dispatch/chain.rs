use crate::dispatch::progress::Progress;
use crate::dispatch::seat::Seat;
use crate::dispatch::{Places, find_runner, vendor};
use crate::error::GateError;
use crate::spec::agents::{Agents, Rung, Runner};

pub(crate) struct Link<'agents> {
    pub(crate) index: usize,
    pub(crate) runner: &'agents Runner,
    pub(crate) rung: &'agents Rung,
    // the rung's place among its own vendor's rungs, so a task changes vendor at the same strength
    level: usize,
}

pub(crate) struct Chain<'agents> {
    pub(crate) agents: &'agents Agents,
    pub(crate) links: Vec<Link<'agents>>,
}

impl<'agents> Chain<'agents> {
    pub(crate) fn new(agents: &'agents Agents) -> Result<Self, GateError> {
        let mut links: Vec<Link<'agents>> = Vec::new();
        for (index, rung) in agents.roles.worker.ladder.iter().enumerate() {
            let runner = find_runner(agents, &rung.runner)?;
            let level = links
                .iter()
                .filter(|link| link.runner.vendor == runner.vendor)
                .count();
            links.push(Link {
                index,
                runner,
                rung,
                level,
            });
        }
        Ok(Self { agents, links })
    }

    // a vendor with fewer rungs takes the task on its top rung
    fn across(&self, progress: &Progress, from: &Link<'_>) -> Vec<&Link<'agents>> {
        let ladder = &self.agents.roles.worker.ladder;
        self.links
            .iter()
            .filter(|link| {
                let top = self
                    .links
                    .iter()
                    .filter(|other| other.runner.vendor == link.runner.vendor)
                    .map(|other| other.level)
                    .max()
                    .unwrap_or(0);
                link.runner.vendor != from.runner.vendor
                    && link.level >= from.level.min(top)
                    && progress.open(ladder, link.index)
            })
            .collect()
    }

    pub(crate) fn next_rung(&self, progress: &Progress, stuck: bool) -> usize {
        let ladder = &self.agents.roles.worker.ladder;
        let Some(here) = self.links.get(progress.rung) else {
            return self.links.len();
        };
        let stay = progress.open(ladder, here.index).then_some(here);
        // a rung that a stand-in start skipped gets its turn after the last one
        let next = self
            .links
            .iter()
            .skip(here.index.saturating_add(1))
            .chain(
                self.links
                    .iter()
                    .take(here.index)
                    .filter(|link| progress.starts.get(link.index) == Some(&0)),
            )
            .find(|link| progress.open(ladder, link.index));
        let order = if stuck { [next, stay] } else { [stay, next] };
        order
            .into_iter()
            .flatten()
            .next()
            .map_or(self.links.len(), |link| link.index)
    }

    pub(crate) fn pick<'chain>(
        &'chain self,
        places: &Places<'_>,
        progress: &Progress,
        own: &'chain Link<'agents>,
    ) -> Result<Option<&'chain Link<'agents>>, GateError> {
        let (agents, git) = (self.agents, places.git_common_dir);
        let mut stand_ins = Vec::new();
        for link in self.across(progress, own) {
            if !vendor::limited(git, agents, &link.runner.vendor)? {
                stand_ins.push(link);
            }
        }
        // the run goes to another vendor at the same level, so this rung keeps its starts for when its vendor is back
        if vendor::limited(git, agents, &own.runner.vendor)? {
            return Ok(stand_ins.first().copied());
        }
        let account = vendor::find_vendor(agents, &own.runner.vendor)?;
        if !agents.policy.overflow || vendor::seat_free(git, agents, account, Seat::Worker)? {
            return Ok(Some(own));
        }
        for link in stand_ins {
            let other = vendor::find_vendor(agents, &link.runner.vendor)?;
            if vendor::seat_free(git, agents, other, Seat::Worker)? {
                return Ok(Some(link));
            }
        }
        Ok(Some(own))
    }
}
