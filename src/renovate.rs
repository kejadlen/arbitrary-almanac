//! What this binary takes from Almanac's renovate almanac: its providers, the reads a Renovate
//! workflow makes, and the fixer's prompt.
//!
//! The judgement stays upstream. `checks_of`, `trust_pending`, and `update_of` decide what a
//! check suite or a pull request body means, and these reads only fetch what they need and hand
//! the answer to a script.

use crate::catalog::{Prompt, Read};
use almanac::agent::Template;
use almanac::cooled::Published;
use almanac::cx::{Cx, StepError};
use almanac::taint::{Sha, Slug, Untrusted};
use almanac_renovate::events::PATCH_TRAILER;
use almanac_renovate::fleet::{Checks, checks_of, trust_pending};
use almanac_renovate::update::{Ages, PullText, update_of};
use almanac_renovate::workflows::build_failures::{FixerPrompt, MIN_RELEASE_AGE, ScanTarget};
use futures::future::BoxFuture;
use github::client::GithubClient;
use serde::de::DeserializeOwned;
use serde_json::{Value as Data, json};

/// The tail of a failed job's log that a fixer reads, where a build's error usually is.
const LOG_LIMIT: usize = 256 * 1024;

pub fn reads() -> Vec<Read> {
    vec![
        Read::of("github.pull", pull),
        Read::of("github.logs", logs),
        Read::cooldown("ages.cooled", MIN_RELEASE_AGE, published),
    ]
}

pub fn prompts() -> Vec<Prompt> {
    vec![Prompt::fixer::<Fixer>("fixer")]
}

/// A read's arguments, as the tuple it takes.
fn arguments<T: DeserializeOwned>(read: &str, args: Vec<Data>) -> Result<T, StepError> {
    serde_json::from_value(Data::from(args)).map_err(|e| StepError(format!("({read} ...) {e}")))
}

almanac::observed! {
    /// What GitHub says now about the Renovate pull request on a branch. A payload is only a hint
    /// that something changed.
    pub struct Pull {
        pub found: bool,
        pub number: u64,
        pub head: Option<Sha>,
        pub conflicted: bool,
        pub failed: bool,
        pub running: bool,
        // A repair of Almanac's is held, and its checks have passed.
        pub repair_passed: bool,
        pub text: Untrusted<PullText>,
    }
}

/// `(github.pull repo branch)`.
fn pull(cx: &Cx, args: Vec<Data>) -> BoxFuture<'_, Result<Pull, StepError>> {
    Box::pin(async move {
        let (repo, branch): (Slug, Slug) = arguments("github.pull", args)?;
        let client = cx.get::<GithubClient>()?;
        let target = cx.get::<ScanTarget>()?;
        let (repo, branch) = (repo.to_string(), branch.to_string());
        let listed = client.pull_for_branch(&repo, &branch).await?;
        // A branch name proves nothing: only Renovate's own pull requests are ours to touch.
        let listed =
            listed.filter(|pull| pull.pointer("/user/login") == Some(&json!(target.author)));
        let Some(number) = listed.and_then(|pull| pull.get("number")?.as_u64()) else {
            return Ok(Pull {
                found: false,
                number: 0,
                head: None,
                conflicted: false,
                failed: false,
                running: false,
                repair_passed: false,
                text: Untrusted::new(PullText::default()),
            });
        };
        let pull = client.pull(&repo, number).await?;
        let head: Sha = pull
            .pointer("/head/sha")
            .and_then(Data::as_str)
            .unwrap_or_default()
            .parse()?;
        let sha = head.to_string();
        let suites = client.check_suites(&repo, &sha).await?;
        let statuses = client.combined_status(&repo, &sha).await?;
        let checks = checks_of(&suites, &statuses);
        let repair_passed = match trust_pending(&statuses) && checks == Checks::Green {
            true => client.pull_commit_messages(&repo, number).await?,
            false => vec![],
        }
        .iter()
        .any(|message| message.contains(PATCH_TRAILER));
        Ok(Pull {
            found: true,
            number,
            head: Some(head),
            conflicted: pull.get("mergeable") == Some(&json!(false)),
            failed: checks == Checks::Red,
            running: checks == Checks::Pending,
            repair_passed,
            text: Untrusted::new(PullText {
                body: pull.get("body").and_then(Data::as_str).map(str::to_string),
            }),
        })
    })
}

almanac::observed! {
    pub struct Logs {
        pub text: Untrusted<String>,
    }
}

/// The last `LOG_LIMIT` bytes of `log`, starting on a character.
fn tail(log: &str) -> &str {
    let start = log.len().saturating_sub(LOG_LIMIT);
    let start = (start..log.len())
        .find(|&at| log.is_char_boundary(at))
        .unwrap_or(0);
    log.get(start..).unwrap_or_default()
}

/// `(github.logs repo sha)`: the tail of each failed check run's log. A run that passed adds
/// nothing, and neither does a log that cannot be read.
fn logs(cx: &Cx, args: Vec<Data>) -> BoxFuture<'_, Result<Logs, StepError>> {
    Box::pin(async move {
        let (repo, sha): (Slug, Sha) = arguments("github.logs", args)?;
        let client = cx.get::<GithubClient>()?;
        let repo = repo.to_string();
        let runs = client.check_runs(&repo, &sha.to_string()).await?;
        let mut text = String::new();
        let failed = runs
            .get("check_runs")
            .and_then(Data::as_array)
            .into_iter()
            .flatten()
            .filter(|run| run.get("conclusion") == Some(&json!("failure")))
            .filter_map(|run| run.get("id")?.as_u64());
        for id in failed {
            if let Ok(log) = client.job_logs(&repo, id).await {
                text.push_str(tail(&log));
                text.push('\n');
            }
        }
        Ok(Logs {
            text: Untrusted::new(text),
        })
    })
}

/// `(ages.cooled text)`: the newest publish time of what a pull request's text says it updates.
fn published(cx: &Cx, args: Vec<Data>) -> BoxFuture<'_, Result<Published, StepError>> {
    Box::pin(async move {
        let (text,): (Untrusted<PullText>,) = arguments("ages.cooled", args)?;
        let ages = cx.get::<dyn Ages>()?;
        ages.newest(&text.map(update_of)).await
    })
}

/// The renovate almanac's fixer prompt, filled from a script's `{:repo ...}`.
#[derive(serde::Deserialize)]
struct Fixer {
    repo: Slug,
}

impl Template for Fixer {
    const SOURCE: &'static str = FixerPrompt::SOURCE;
    const VARS: &'static [&'static str] = FixerPrompt::VARS;

    fn render(&self) -> String {
        let repo = self.repo.clone();
        FixerPrompt { repo }.render()
    }
}

#[cfg(test)]
#[path = "renovate_tests.rs"]
mod tests;
