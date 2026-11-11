//! What this binary ships for workflows to use: the events a workflow may run on, the reads
//! `gather` may be handed, and the prompts its agents run.

use almanac::agent::{AgentSpec, Fixer, Quarantined, Template};
use almanac::cooled::{Age, Cooled, Published};
use almanac::cx::{Cx, Halt, Record, StepError};
use almanac::patch::Patch;
use almanac::taint::{Observed, Sha, Slug, Untrusted};
use almanac::wire::Malformed;
use almanac::workflow::{Found, Registered, Scanner, Subject};
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value as Data, json};
use std::sync::Arc;
use std::time::Duration;

/// Everything a workflow may name. An event, provider, or read not here refuses the workflow at
/// load or ends its run.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub events: Vec<Event>,
    pub reads: Vec<Read>,
    pub prompts: Vec<Prompt>,
    pub scans: Vec<Scan>,
}

impl Catalog {
    /// What this binary ships.
    pub fn shipped() -> Self {
        Self {
            events: vec![crate::renovate::build_changed()],
            reads: crate::renovate::reads(),
            prompts: crate::renovate::prompts(),
            scans: crate::renovate::scans(),
        }
    }

    pub fn event(&self, kind: &str) -> Option<&Event> {
        self.events.iter().find(|event| event.kind == kind)
    }

    pub fn scan(&self, name: &str) -> Option<&Scan> {
        self.scans.iter().find(|scan| scan.name == name)
    }

    pub fn prompt(&self, name: &str) -> Option<&Prompt> {
        self.prompts.iter().find(|prompt| prompt.name == name)
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

/// What a read answers: data for the script, and, from a cooldown, the proof that an update has
/// cooled. The proof stays in Rust, where only a fixer's call can use it.
pub struct Answer {
    pub data: Data,
    pub cooled: Option<Cooled>,
}

type Perform =
    dyn for<'a> Fn(&'a Cx, Vec<Data>) -> BoxFuture<'a, Result<Answer, StepError>> + Send + Sync;

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
                    Ok(Answer {
                        data: serde_json::to_value(answer)?,
                        cooled: None,
                    })
                })
            }),
        }
    }

    /// A read of whether an update has aged past `min_age`, from a publish time `fetch` reads.
    /// The script sees `{:age :passed}`, `:young`, or `:unknown`. Almanac records the publish
    /// time and the moment of the check, so a replay reaches the same answer.
    pub fn cooldown(name: &'static str, min_age: Duration, fetch: Fetch<Published>) -> Self {
        Self {
            name,
            perform: Arc::new(move |cx, args| {
                Box::pin(async move {
                    let (age, cooled) = match cx.age(min_age, fetch(cx, args)).await? {
                        Age::Passed(cooled) => ("passed", Some(cooled)),
                        Age::Young => ("young", None),
                        Age::Unknown => ("unknown", None),
                    };
                    Ok(Answer {
                        data: json!({ "age": age }),
                        cooled,
                    })
                })
            }),
        }
    }

    /// `(run.attempts)`: how many patches this run's key has had recently, as `{:used n :asked
    /// bool}`, where `asked` says a person was already asked. Almanac records the count.
    pub fn attempts(name: &'static str) -> Self {
        Self {
            name,
            perform: Arc::new(move |cx, args| {
                Box::pin(async move {
                    let []: [Data; 0] = arguments(name, args)?;
                    let attempts = cx.attempts().await?;
                    Ok(Answer {
                        data: json!({"used": attempts.used, "asked": attempts.asked}),
                        cooled: None,
                    })
                })
            }),
        }
    }

    /// `(run.hold repo sha)`: a `pending` status under `context` on a commit now, before an agent
    /// starts, so nothing merges while it works. Almanac posts it only in a live run.
    pub fn hold(name: &'static str, context: &'static str) -> Self {
        Self {
            name,
            perform: Arc::new(move |cx, args| {
                Box::pin(async move {
                    let (repo, sha): (Slug, Sha) = arguments(name, args)?;
                    cx.hold(context, &repo, &sha).await?;
                    Ok(Answer {
                        data: Data::Null,
                        cooled: None,
                    })
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
    ) -> BoxFuture<'a, Result<Answer, StepError>> {
        (self.perform)(cx, args)
    }
}

type Invoke = dyn for<'a> Fn(
        &'a Cx,
        &'static str,
        Data,
        Data,
        Option<&'a Cooled>,
    ) -> BoxFuture<'a, Result<Data, StepError>>
    + Send
    + Sync;

/// A prompt an agent may run, with the profile that fixes what the agent may do. A workflow's
/// `:agents` names one by `:prompt`. Its answer is untrusted, so it reaches a script as an opaque
/// handle.
#[derive(Clone)]
pub struct Prompt {
    pub name: &'static str,
    /// What the script calls: `read` for a quarantined reader, `repair` for a fixer.
    pub method: &'static str,
    pub vars: &'static [&'static str],
    invoke: Arc<Invoke>,
}

impl Prompt {
    /// A quarantined reader of hostile content: no writes, no network, no secrets.
    pub fn quarantined<T: Template + DeserializeOwned>(name: &'static str) -> Self {
        Self {
            name,
            method: "read",
            vars: T::VARS,
            invoke: Arc::new(|cx, agent, vars, input, _| {
                Box::pin(async move {
                    let spec = AgentSpec::<Quarantined, T>::quarantined(agent);
                    let prompt: T = serde_json::from_value(vars)?;
                    let answer: Untrusted<Data> =
                        cx.agent(spec).read(prompt, &input).await.map_err(step)?;
                    Ok(serde_json::to_value(answer)?)
                })
            }),
        }
    }

    /// A fixer, which writes inside a sandbox. It reads an update only after a cooldown read in
    /// the same run has passed. It answers with `{:patch :diff :said}`: the patch to push, and
    /// for a person asked to take over, its diff and what the agent said, by `said`.
    pub fn fixer<T: Template + DeserializeOwned>(
        name: &'static str,
        said: fn(&Patch) -> String,
    ) -> Self {
        Self {
            name,
            method: "repair",
            vars: T::VARS,
            invoke: Arc::new(move |cx, agent, vars, input, cooled| {
                Box::pin(async move {
                    let Some(cooled) = cooled else {
                        return Err(StepError(format!(
                            "{agent} reads an update only after a cooldown read has passed"
                        )));
                    };
                    let spec = AgentSpec::<Fixer, T>::fixer(agent);
                    let prompt: T = serde_json::from_value(vars)?;
                    let patch: Untrusted<Patch> = cx
                        .agent(spec)
                        .repair(prompt, &input, cooled)
                        .await
                        .map_err(step)?;
                    let diff = patch.map(|patch| patch.diff.clone());
                    Ok(json!({"patch": patch, "diff": diff, "said": patch.map(said)}))
                })
            }),
        }
    }

    pub fn invoke<'a>(
        &self,
        cx: &'a Cx,
        agent: &'static str,
        vars: Data,
        input: Data,
        cooled: Option<&'a Cooled>,
    ) -> BoxFuture<'a, Result<Data, StepError>> {
        (self.invoke)(cx, agent, vars, input, cooled)
    }
}

/// A scan a workflow may name in `:scan`, which finds the events a webhook missed or a later
/// look must see, such as an update that has since cooled. It is the scan of one of Almanac's
/// Rust workflows, so what deserves a run is decided upstream.
#[derive(Clone)]
pub struct Scan {
    pub name: &'static str,
    finder: Arc<dyn Registered>,
}

impl Scan {
    pub fn of(name: &'static str, finder: impl Registered + 'static) -> Self {
        Self {
            name,
            finder: Arc::new(finder),
        }
    }

    /// The kind of event the scan finds, which must be the kind its workflow runs on.
    pub fn kind(&self) -> &'static str {
        self.finder.kind()
    }

    pub fn find<'a>(
        &'a self,
        scanner: &'a Scanner,
    ) -> BoxFuture<'a, Result<Vec<Found>, StepError>> {
        self.finder.scan(scanner)
    }
}

/// A read's arguments, as the tuple it takes.
pub(crate) fn arguments<T: DeserializeOwned>(read: &str, args: Vec<Data>) -> Result<T, StepError> {
    serde_json::from_value(Data::from(args)).map_err(|e| StepError(format!("({read} ...) {e}")))
}

/// An agent's call fails only as a step: it never decides to stop.
fn step(halt: Halt) -> StepError {
    let Halt::Failed(error) = halt else {
        unreachable!("an agent call only fails")
    };
    error
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind)
    }
}

impl std::fmt::Debug for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

impl std::fmt::Debug for Scan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

impl std::fmt::Debug for Read {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}
