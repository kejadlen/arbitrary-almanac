//! A workflow as Almanac's runtime holds it.
//!
//! `Registered` is the erased form the runtime runs, so routing, limits, the run log, and replay
//! are Almanac's own. Each read `gather` makes goes through the run's `Cx`, which records it, so a
//! replay answers from the record and reads nothing again.

use super::Workflow;
use super::gather::{Call, Dispatch};
use super::policy::Decision;
use crate::catalog::Read;
use almanac::cx::{Cx, Halt, StepError};
use almanac::effect::{Effect, EffectKind};
use almanac::providers::Requirement;
use almanac::taint::Title;
use almanac::wire::Malformed;
use almanac::workflow::{Decided, Found, Identity, Registered, Scanner};
use futures::future::BoxFuture;
use serde_json::{Value as Data, json};
use std::collections::BTreeMap;
use std::future::Future;

/// A loaded workflow with the names the runtime keeps for as long as the binary runs.
pub struct Registration {
    workflow: Workflow,
    name: &'static str,
    intent: &'static str,
    may: &'static [EffectKind],
    agents: Vec<&'static str>,
    /// The workflow's string literals, which an effect's `&'static` names are drawn from.
    literals: BTreeMap<String, &'static str>,
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
            literals: workflow
                .literals
                .iter()
                .map(|text| (text.clone(), leak(text)))
                .collect(),
            workflow,
        }
    }
}

impl Registration {
    fn effect(&self, decision: Decision) -> Effect {
        // `policy` refuses a name that is not one of the workflow's literals.
        let literal = |text: &str| self.literals.get(text).copied().unwrap_or_default();
        match decision {
            Decision::Attest {
                context,
                state,
                repo,
                sha,
            } => Effect::Attest {
                context: literal(&context),
                state,
                repo,
                sha,
            },
            Decision::Await { until } => Effect::Await {
                until: literal(&until),
            },
            // Every Lua string is clean, since untrusted text reaches a script only as a handle.
            Decision::File {
                title,
                target,
                body,
            } => Effect::File {
                title: Title::__compose(title),
                target,
                body,
            },
            Decision::Ask {
                question,
                target,
                pull,
                body,
            } => Effect::Ask {
                question: Title::__compose(question),
                target,
                pull,
                body,
            },
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
            let facts = self
                .workflow
                .gather(&reading.data, &mut recorded)
                .await
                .map_err(failed)?;
            let decisions = self.workflow.policy(&facts).map_err(failed)?;
            Ok(Decided {
                effects: decisions.into_iter().map(|d| self.effect(d)).collect(),
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
