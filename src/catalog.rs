//! What this binary ships for workflows to use: the events a workflow may run on and the reads
//! `gather` may be handed. Prompts join it as the host grows.

use almanac::cx::{Cx, Record, StepError};
use almanac::taint::Observed;
use almanac::wire::Malformed;
use almanac::workflow::Subject;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::Value as Data;
use std::sync::Arc;

/// Everything a workflow may name. An event, provider, or read not here refuses the workflow at
/// load or ends its run.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub events: Vec<Event>,
    pub reads: Vec<Read>,
}

impl Catalog {
    /// What this binary ships.
    pub fn shipped() -> Self {
        Self {
            events: vec![check_suite_completed()],
            reads: vec![],
        }
    }

    pub fn event(&self, kind: &str) -> Option<&Event> {
        self.events.iter().find(|event| event.kind == kind)
    }

    /// Whether any read belongs to the provider `name`, such as `github` for `github.pull`.
    pub fn provides(&self, name: &str) -> bool {
        self.reads.iter().any(|read| read.provider() == name)
    }
}

/// An event as a run sees it: its data, the key that keeps one run active per subject, and the
/// subjects it links.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub data: Data,
    pub key: String,
    pub subjects: Vec<Subject>,
}

type ReadEvent = dyn Fn(&Data) -> Result<Reading, Malformed> + Send + Sync;

/// An event a workflow may name in `:on`.
#[derive(Clone)]
pub struct Event {
    pub kind: &'static str,
    read: Arc<ReadEvent>,
}

impl Event {
    /// An event read by `read`, which must project the payload down to clean values and
    /// `Untrusted` text.
    pub fn new(
        kind: &'static str,
        read: impl Fn(&Data) -> Result<Reading, Malformed> + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            read: Arc::new(read),
        }
    }

    /// One of Almanac's events. Its `Observed` fields are clean or `Untrusted`, so its JSON is
    /// safe to hand a script.
    pub fn of<E: almanac::wire::Event + Serialize>(
        key: fn(&E) -> String,
        subjects: fn(&E) -> Vec<Subject>,
    ) -> Self {
        Self::new(E::KIND, move |payload| {
            let event = E::from_json(payload)?;
            let Ok(data) = serde_json::to_value(&event) else {
                unreachable!("an Almanac event serializes, since it was read from JSON")
            };
            Ok(Reading {
                data,
                key: key(&event),
                subjects: subjects(&event),
            })
        })
    }

    pub fn read(&self, payload: &Data) -> Result<Reading, Malformed> {
        (self.read)(payload)
    }
}

type Perform =
    dyn for<'a> Fn(&'a Cx, Vec<Data>) -> BoxFuture<'a, Result<Data, StepError>> + Send + Sync;

/// How a read fetches its answer from the run's providers.
pub type Fetch<T> = for<'a> fn(&'a Cx, Vec<Data>) -> BoxFuture<'a, Result<T, StepError>>;

/// A read `gather` may make, such as `github.pull`. Its answer is recorded under its name, so a
/// replay answers from the record and reads nothing.
#[derive(Clone)]
pub struct Read {
    pub name: &'static str,
    perform: Arc<Perform>,
}

impl Read {
    /// A read named `<provider>.<method>` whose answer is `T`. `T` is `Observed`, so its text is
    /// `Untrusted` and reaches a script as an opaque handle.
    pub fn of<T: Observed + Record>(name: &'static str, fetch: Fetch<T>) -> Self {
        Self {
            name,
            perform: Arc::new(move |cx, args| {
                Box::pin(async move {
                    let answer = cx.observe(name, fetch(cx, args)).await?;
                    Ok(serde_json::to_value(answer)?)
                })
            }),
        }
    }

    pub fn provider(&self) -> &str {
        self.name
            .split_once('.')
            .map_or(self.name, |(provider, _)| provider)
    }

    pub fn method(&self) -> &str {
        self.name.split_once('.').map_or("", |(_, method)| method)
    }

    pub fn perform<'a>(
        &self,
        cx: &'a Cx,
        args: Vec<Data>,
    ) -> BoxFuture<'a, Result<Data, StepError>> {
        (self.perform)(cx, args)
    }
}

/// One run per branch, since a fix for a red build lands on the branch, not the commit.
fn check_suite_completed() -> Event {
    use github::CheckSuiteCompleted;
    Event::of::<CheckSuiteCompleted>(
        |suite| match &suite.head_branch {
            Some(branch) => format!("{}:{branch}", suite.repo),
            None => format!("{}:{}", suite.repo, suite.head_sha),
        },
        |suite| vec![Subject::repo(&suite.repo)],
    )
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind)
    }
}

impl std::fmt::Debug for Read {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}
