use super::*;
use crate::Catalog;
use crate::host::{Registration, Workflow};
use almanac::prelude::Providers;
use almanac::runtime::{Envelope, Origin, Outcome, Runtime};
use almanac::store::MemoryStore;
use almanac::testing::ScriptedHarness;
use almanac_renovate::update::Update;
use github::client::Auth;
use simforge::{RENOVATE, SimForge, Suite};
use std::sync::Arc;

/// Every update was published at the same time.
struct Fixed(Published);

impl Ages for Fixed {
    fn newest<'a>(
        &'a self,
        _: &'a Untrusted<Update>,
    ) -> BoxFuture<'a, Result<Published, StepError>> {
        Box::pin(async move { Ok(self.0) })
    }
}

fn days_ago(days: i64) -> Published {
    let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(days * 24);
    serde_json::from_value(json!({ "At": at.to_string() })).unwrap()
}

/// A workflow on check suites with the shipped catalog, that files what it gathered.
fn reporting(gather: &str, may: &str, agents: &str) -> Registration {
    let source = format!(
        r#"(local declare {{:name :red :on :github.check_suite.completed :intent "Report"
                         :may [{may}] :runs-per-hour 10 :agents {{{agents}}}}})
{gather}
(fn policy [facts {{: file}}]
  [(file {{:title facts.said :target facts.repo :body facts.body}})])
{{: declare : gather : policy}}"#
    );
    Registration::new(Workflow::load(&Catalog::shipped(), "red.fnl", &source).unwrap())
}

const REPORT: &str = r#"(fn gather [{: github : ages} event]
  (let [pull (github.pull event.repo event.head_branch)]
    (if (not pull.found)
        {:repo event.repo :said "missing" :body ""}
        {:repo event.repo
         :said (.. (. (ages.cooled pull.text) :age)
                   (if pull.failed " red" "") (if pull.running " running" "")
                   (if pull.conflicted " conflicted" "") (if pull.repair_passed " repaired" ""))
         :body (. (github.logs event.repo pull.head) :text)})))"#;

fn suite(sha: &str) -> Data {
    json!({
        "repository": {"full_name": "o/r"},
        "check_suite": {"head_branch": "renovate/x", "head_sha": sha, "conclusion": "failure",
                        "id": 1, "pull_requests": []},
        "sender": {"login": "bot"},
    })
}

fn providers(forge: &SimForge, published: Published) -> Providers {
    let client = GithubClient::new(forge.base_url(), Auth::Token("t".into()));
    let target = ScanTarget {
        owner: "o".into(),
        author: RENOVATE.into(),
    };
    let ages: Arc<dyn Ages> = Arc::new(Fixed(published));
    Providers::new()
        .provide(Arc::new(client))
        .provide(Arc::new(target))
        .provide::<dyn Ages>(ages)
}

/// Runs `workflow` on a check suite for `sha` and returns the one effect it filed, as
/// `(title, body)`, or else the outcomes, which hold the error it failed with.
async fn report(workflow: Registration, providers: Providers, sha: &str) -> Result<Data, String> {
    let runtime = Runtime::new(vec![Box::new(workflow)], providers)
        .with_store(Arc::new(MemoryStore::default()));
    let envelope = Envelope {
        source: "github".into(),
        delivery_id: "d1".into(),
        kind: "github.check_suite.completed".into(),
        depth: 0,
        origin: Origin::Webhook,
        payload: suite(sha),
    };
    runtime.ingest(envelope).await.unwrap();
    match runtime.drain().await.unwrap().as_slice() {
        [Outcome::Ran { effects, .. }] => {
            let planned: Vec<Data> = effects
                .iter()
                .map(|e| serde_json::to_value(almanac::effect::Planned::from(e)).unwrap())
                .collect();
            Ok(json!([
                planned[0]["File"]["title"],
                planned[0]["File"]["body"]
            ]))
        }
        // A failed run, with its error.
        other => Err(format!("{other:?}")),
    }
}

/// What a forge holds for a case: the pull request's author, its mergeability, its suite's
/// status and conclusion, and whether a repair of Almanac's is held on it.
struct Case {
    author: &'static str,
    mergeable_state: &'static str,
    suite: (&'static str, Option<&'static str>),
    held: bool,
    published: Published,
    expected: Data,
}

#[tokio::test]
async fn github_and_ages_read_a_renovate_pull_request_as_it_is_now() {
    let red = ("completed", Some("failure"));
    let cases = [
        Case {
            author: RENOVATE,
            mergeable_state: "clean",
            suite: red,
            held: false,
            published: days_ago(10),
            expected: json!(["passed red", {"$untrusted": "boom\n"}]),
        },
        Case {
            author: "someone",
            mergeable_state: "clean",
            suite: red,
            held: false,
            published: days_ago(10),
            expected: json!(["missing", {"$untrusted": ""}]),
        },
        Case {
            author: RENOVATE,
            mergeable_state: "dirty",
            suite: ("in_progress", None),
            held: false,
            published: days_ago(0),
            expected: json!(["young running conflicted", {"$untrusted": ""}]),
        },
        Case {
            author: RENOVATE,
            mergeable_state: "clean",
            suite: ("completed", Some("success")),
            held: true,
            published: Published::Unknown,
            expected: json!(["unknown repaired", {"$untrusted": ""}]),
        },
        // A repair passed only while Almanac holds it.
        Case {
            author: RENOVATE,
            mergeable_state: "clean",
            suite: ("completed", Some("success")),
            held: false,
            published: days_ago(10),
            expected: json!(["passed", {"$untrusted": ""}]),
        },
    ];
    for case in cases {
        let forge = SimForge::start().await;
        forge.add_repo(simforge::Repo::public("o/r"));
        let mut pull = simforge::Pull::renovate("renovate/x", "Update x");
        pull.author = case.author.into();
        pull.mergeable_state = case.mergeable_state.into();
        pull.commits = vec![format!("Repair\n\n{PATCH_TRAILER} k")];
        let number = forge.add_pull("o/r", pull);
        let sha = forge.pull("o/r", number).head_sha;
        let (status, conclusion) = case.suite;
        let mut suite = Suite::finished(&sha, "renovate/x", conclusion.unwrap_or_default(), "boom");
        suite.status = status.into();
        suite.conclusion = conclusion.map(String::from);
        forge.add_suite("o/r", suite);
        if case.held {
            let client = GithubClient::new(forge.base_url(), Auth::Token("t".into()));
            let held = json!({"context": "almanac/trust", "state": "pending"});
            client
                .post(&format!("/repos/o/r/statuses/{sha}"), &held)
                .await
                .unwrap();
        }
        let workflow = reporting(REPORT, ":file", "");
        let reported = report(workflow, providers(&forge, case.published), &sha).await;
        assert_eq!(reported, Ok(case.expected.clone()), "{}", case.expected);
    }
}

#[tokio::test]
async fn a_read_refuses_arguments_it_cannot_take() {
    let cases = [
        (
            "(github.pull event.repo)",
            "(github.pull ...) invalid length 1",
        ),
        ("(github.logs event.repo :main)", "(github.logs ...) "),
        ("(ages.cooled \"text\")", "(ages.cooled ...) "),
    ];
    let forge = SimForge::start().await;
    forge.add_repo(simforge::Repo::public("o/r"));
    for (call, error) in cases {
        let gather = format!("(fn gather [{{: github : ages}} event] {call})");
        let workflow = reporting(&gather, ":file", "");
        let providers = providers(&forge, Published::Unknown);
        let reported = report(workflow, providers, &"a".repeat(40)).await;
        let reported = reported.unwrap_err();
        assert!(reported.contains(error), "{call}: {reported}");
    }
}

#[test]
fn a_log_is_cut_to_its_tail_on_a_character() {
    let long = format!("{}é{}", "a".repeat(10), "b".repeat(LOG_LIMIT - 1));
    let cases = [
        ("short".to_string(), "short".to_string()),
        // A fixer reads the last 256 KiB.
        ("b".repeat(300_000), "b".repeat(262_144)),
        // The cut falls inside `é`, so the tail starts after it.
        (long, "b".repeat(LOG_LIMIT - 1)),
    ];
    for (log, expected) in cases {
        assert_eq!(tail(&log), expected);
    }
}

#[tokio::test]
async fn the_fixer_runs_the_renovate_almanacs_prompt() {
    let forge = SimForge::start().await;
    forge.add_repo(simforge::Repo::public("o/r"));
    let gather = r#"(fn gather [{: github : ages : fixer} event]
  (ages.cooled (. (github.pull event.repo event.head_branch) :text))
  (fixer.repair {:repo event.repo} {:logs ""})
  {:repo event.repo :said "fixed" :body ""})"#;
    let workflow = reporting(gather, ":file", ":fixer {:prompt :fixer :vars [:repo]}");
    let harness = ScriptedHarness::replying(r#"{"changes": [], "diff": ""}"#);
    let providers = providers(&forge, days_ago(10)).harness(harness.clone());
    let reported = report(workflow, providers, &"a".repeat(40)).await;
    assert_eq!(reported, Ok(json!(["fixed", {"$untrusted": ""}])));
    let prompts = harness.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].starts_with("You repair a broken dependency update in `o/r`."),
        "{prompts:?}"
    );
}
