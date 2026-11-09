//! A workflow as Almanac's runtime holds it.
//!
//! `Registered` is the erased form the runtime runs, so routing, limits, the run log, and replay
//! are Almanac's own. Each read `gather` makes goes through the run's `Cx`, which records it, so a
//! replay answers from the record and reads nothing again.

use super::Workflow;
use super::gather::{Call, Dispatch};
use crate::catalog::Read;
use almanac::cx::{Cx, Halt, StepError};
use almanac::effect::EffectKind;
use almanac::providers::Requirement;
use almanac::wire::Malformed;
use almanac::workflow::{Decided, Found, Identity, Registered, Scanner};
use futures::future::BoxFuture;
use serde_json::{Value as Data, json};
use std::future::Future;

/// A loaded workflow with the names the runtime keeps for as long as the binary runs.
pub struct Registration {
    workflow: Workflow,
    name: &'static str,
    intent: &'static str,
    may: &'static [EffectKind],
    agents: Vec<&'static str>,
}

impl Registration {
    /// The runtime holds its workflows until the binary exits, so their names are leaked once,
    /// at startup.
    pub fn new(workflow: Workflow) -> Self {
        let declaration = &workflow.declaration;
        let leak = |text: &str| -> &'static str { Box::leak(text.into()) };
        Self {
            name: leak(&declaration.name),
            intent: leak(&declaration.intent),
            may: Box::leak(declaration.may.clone().into_boxed_slice()),
            agents: declaration.agents.keys().map(|name| leak(name)).collect(),
            workflow,
        }
    }
}

/// Performs each call through the run's `Cx`.
struct Recorded<'a> {
    cx: &'a Cx,
    reads: &'a [Read],
}

impl Dispatch for Recorded<'_> {
    fn call(&mut self, call: Call) -> impl Future<Output = Result<Data, String>> + Send {
        let read = self
            .reads
            .iter()
            .find(|read| read.provider() == call.capability && read.method() == call.method);
        let missing = format!("{} has no read `{}`", call.capability, call.method);
        let performed = read.map(|read| read.perform(self.cx, call.args));
        async move {
            match performed {
                Some(performed) => performed.await.map_err(|StepError(message)| message),
                None => Err(missing),
            }
        }
    }
}

impl Registered for Registration {
    fn name(&self) -> &'static str {
        self.name
    }

    fn intent(&self) -> &'static str {
        self.intent
    }

    fn kind(&self) -> &'static str {
        self.workflow.event.kind
    }

    fn handles(&self, kind: &str) -> bool {
        kind == self.workflow.event.kind
    }

    fn runs_per_hour(&self) -> u32 {
        self.workflow.declaration.runs_per_hour
    }

    fn may(&self) -> &'static [EffectKind] {
        self.may
    }

    fn agent_names(&self) -> Vec<&'static str> {
        self.agents.clone()
    }

    fn requirements(&self) -> Vec<Requirement> {
        vec![]
    }

    fn identify(&self, payload: &Data) -> Result<Identity, Malformed> {
        let reading = self.workflow.event.read(payload)?;
        Ok(Identity {
            key: reading.key,
            subjects: reading.subjects,
            wanted: true,
        })
    }

    fn execute<'a>(
        &'a self,
        cx: &'a Cx,
        payload: &'a Data,
    ) -> BoxFuture<'a, Result<Decided, Halt>> {
        Box::pin(async move {
            let reading = self.workflow.event.read(payload).map_err(StepError::from)?;
            let mut recorded = Recorded {
                cx,
                reads: &self.workflow.reads,
            };
            let failed = |error: crate::Error| StepError(error.to_string());
            self.workflow
                .gather(&reading.data, &mut recorded)
                .await
                .map_err(failed)?;
            // Policy comes next. Until then a run decides nothing.
            Ok(Decided {
                effects: vec![],
                links: vec![],
            })
        })
    }

    fn scan<'a>(&'a self, _: &'a Scanner) -> BoxFuture<'a, Result<Vec<Found>, StepError>> {
        Box::pin(async { Ok(vec![]) })
    }

    fn manifest(&self) -> Data {
        json!({
            "workflow": self.name(),
            "intent": self.intent(),
            "on": self.kind(),
            "scans": false,
            "runs_per_hour": self.runs_per_hour(),
            "may": self.may(),
            "agents": self.agent_names(),
            "capabilities": self.workflow.capabilities,
        })
    }
}
