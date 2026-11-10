use super::testing::{Ages, catalog};
use super::*;
use almanac::agent::{Fixer, Profile as _, Quarantined};
use almanac::effect::{EffectKind, Planned};
use almanac::prelude::{Providers, Slug};
use almanac::runtime::{Envelope, Origin, Outcome, Runtime};
use almanac::store::MemoryStore;
use almanac::workflow::{Registered, Subject};
use serde_json::{Value as Data, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// A workflow that runs on `on` and whose gather is `gather`.
fn on(on: &str, gather: &str) -> Registration {
    let source = format!(
        r#"(local declare {{:name :red :on {on} :intent "Look at red builds" :may [:attest]
                         :runs-per-hour 10}})
{gather}
(fn policy [facts {{: attest}}])
{{: declare : gather : policy}}"#
    );
    Registration::new(Workflow::load(&catalog(), "red.fnl", &source).unwrap())
}

fn suite(branch: Option<&str>) -> Data {
    json!({
        "repository": {"full_name": "o/r"},
        "check_suite": {"head_branch": branch, "head_sha": "a".repeat(40), "conclusion": "failure",
                        "id": 1, "pull_requests": [{"number": 3}]},
        "sender": {"login": "bot"},
    })
}

fn runtime(workflow: Registration, providers: Providers) -> Runtime {
    Runtime::new(vec![Box::new(workflow)], providers).with_store(Arc::new(MemoryStore::default()))
}

/// Delivers one event and runs what it starts.
async fn run(runtime: &Runtime, kind: &str, payload: Data) -> Vec<Outcome> {
    let envelope = Envelope {
        source: "github".into(),
        delivery_id: "d1".into(),
        kind: kind.into(),
        depth: 0,
        origin: Origin::Webhook,
        payload,
    };
    runtime.ingest(envelope).await.unwrap();
    runtime.drain().await.unwrap()
}

/// The run id and effects of the one run in `outcomes`, if it ran, with each effect as Almanac
/// stores it.
fn ran(outcomes: &[Outcome]) -> Option<(String, Vec<Data>)> {
    match outcomes {
        [
            Outcome::Ran {
                run_id, effects, ..
            },
        ] => Some((
            run_id.clone(),
            effects
                .iter()
                .map(|effect| serde_json::to_value(Planned::from(effect)).unwrap_or_default())
                .collect(),
        )),
        _ => None,
    }
}

/// The run id of the one run in `outcomes`, if it ran and decided nothing.
fn ran_quietly(outcomes: &[Outcome]) -> Option<String> {
    ran(outcomes)
        .filter(|(_, effects)| effects.is_empty())
        .map(|(run_id, _)| run_id)
}

#[tokio::test]
async fn a_run_reads_through_cx_and_a_replay_reads_nothing_again() {
    let ages = Arc::new(Ages::default());
    let workflow = on(
        ":github.check_suite.completed",
        "(fn gather [{: ages : github} event]
           (github.pull event.repo 3)
           (assert (= (. (ages.age event.repo) :calls) 1)))",
    );
    let runtime = runtime(workflow, Providers::new().provide(ages.clone()));

    let outcomes = run(
        &runtime,
        "github.check_suite.completed",
        suite(Some("renovate/x")),
    )
    .await;
    let run_id = ran_quietly(&outcomes);
    assert!(run_id.is_some(), "{outcomes:?}");
    let run_id = run_id.unwrap_or_default();

    // The replay's gather would fail its assert if `ages` were asked again.
    let replayed = runtime.replay(&run_id).await.unwrap();
    assert_eq!(ran_quietly(&[replayed]), Some(run_id.clone()));
    assert_eq!(ages.0.load(Ordering::SeqCst), 1);

    let repo: Slug = "o/r".parse().unwrap();
    let runs = runtime.runs_for(&Subject::repo(&repo)).await.unwrap();
    assert_eq!(runs, vec![run_id]);
}

#[tokio::test]
async fn a_read_that_fails_or_does_not_exist_fails_the_run() {
    let cases = [
        ("(ages.age)", "no provider for"),
        (
            "(github.merge)",
            "gather failed: github has no read `merge`",
        ),
        ("(error :boom)", "gather failed: red.fnl:3: boom"),
    ];
    for (body, error) in cases {
        let workflow = on(
            ":e",
            &format!("(fn gather [{{: ages : github}} event] {body})"),
        );
        let runtime = runtime(workflow, Providers::new());
        let outcomes = run(&runtime, "e", json!({})).await;
        assert_eq!(ran_quietly(&outcomes), None, "{body}");
        let failed: Vec<&str> = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::Failed { error, .. } => Some(error.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(failed.len(), outcomes.len(), "{body}: {outcomes:?}");
        assert!(
            failed.iter().all(|e| e.contains(error)),
            "{body}: {failed:?}"
        );
    }
}

#[test]
fn a_check_suite_is_keyed_by_its_branch_or_else_its_commit() {
    let workflow = on(":github.check_suite.completed", "(fn gather [{} event])");
    let repo: Slug = "o/r".parse().unwrap();
    let cases = [
        (Some("renovate/x"), "o/r:renovate/x".to_string()),
        (None, format!("o/r:{}", "a".repeat(40))),
    ];
    for (branch, key) in cases {
        let identity = workflow.identify(&suite(branch)).unwrap();
        assert_eq!(identity.key, key);
        assert_eq!(identity.subjects, vec![Subject::repo(&repo)]);
        assert!(identity.wanted);
    }
    let pointer = workflow.identify(&json!({})).err().map(|err| err.pointer);
    assert_eq!(pointer.as_deref(), Some("/repository/full_name"));
}

#[tokio::test]
async fn the_runtime_sees_the_declaration_and_the_capabilities() {
    let source = r#"(local declare {:name :red :on :github.check_suite.completed :intent "Look"
                 :may [:attest :push] :runs-per-hour 10 :agents {:fixer {:prompt :look}}})
(fn gather [{: github : fixer} event])
(fn policy [facts {: push}])
{: declare : gather : policy}"#;
    let workflow = Registration::new(Workflow::load(&catalog(), "red.fnl", source).unwrap());
    let runtime = runtime(workflow, Providers::new());
    assert_eq!(
        runtime.manifest(),
        vec![json!({
            "workflow": "red",
            "intent": "Look",
            "on": "github.check_suite.completed",
            "scans": false,
            "runs_per_hour": 10,
            "may": [EffectKind::Attest, EffectKind::Push],
            "agents": ["fixer"],
            "capabilities": {"providers": ["github"], "agents": ["fixer"], "effects": ["Push"]},
        })]
    );
    let missing = runtime.validate().err().map(|err| err.to_string());
    assert!(
        missing
            .as_deref()
            .unwrap_or_default()
            .contains("red needs a harness for agent `fixer`"),
        "{missing:?}"
    );
    assert_eq!(runtime.backfill(None).await.unwrap().found, 0);
}

#[test]
fn a_catalog_names_what_it_ships() {
    assert_eq!(
        format!("{:?}", catalog()),
        "Catalog { events: [github.check_suite.completed, e], reads: [github.pull, ages.age, ages.cooled], prompts: [fix, look] }"
    );
}

#[tokio::test]
async fn a_workflow_runs_only_on_its_event() {
    let workflow = on(":e", "(fn gather [{} event] (error :ran))");
    let runtime = runtime(workflow, Providers::new());
    let outcomes = run(&runtime, "github.check_suite.completed", suite(None)).await;
    assert!(outcomes.is_empty(), "{outcomes:?}");
}

#[tokio::test]
async fn a_run_decides_its_effects_and_a_replay_decides_them_again() {
    let source = r#"(local declare {:name :red :on :github.check_suite.completed :intent "Look"
                 :may [:attest :await :file :ask] :runs-per-hour 10})
(fn gather [{} event] {:repo event.repo :sha event.head_sha})
(fn policy [facts {: attest : await : file : ask}]
  [(attest {:context "almanac/red" :state :failure :repo facts.repo :sha facts.sha})
   (await "checks")
   (file {:title (.. "Red in " facts.repo) :target facts.repo :body "b"})
   (ask {:question "Fix?" :target facts.repo :body "b"})])
{: declare : gather : policy}"#;
    let workflow = Registration::new(Workflow::load(&catalog(), "red.fnl", source).unwrap());
    let runtime = runtime(workflow, Providers::new());
    let sha = "a".repeat(40);
    let expected = vec![
        json!({"Attest": {"context": "almanac/red", "state": "Failure", "repo": "o/r", "sha": sha}}),
        json!({"Await": {"until": "checks"}}),
        json!({"File": {"title": "Red in o/r", "target": "o/r", "body": {"$untrusted": "b"}}}),
        json!({"Ask": {"question": "Fix?", "target": "o/r", "pull": null,
                       "body": {"$untrusted": "b"}}}),
    ];

    let outcomes = run(&runtime, "github.check_suite.completed", suite(None)).await;
    let (run_id, first) = ran(&outcomes).unwrap_or_default();
    assert_eq!(first, expected, "{outcomes:?}");

    let replayed = runtime.replay(&run_id).await.unwrap();
    assert_eq!(ran(&[replayed]), Some((run_id, expected)));
}

/// A workflow on check suites whose agents are a fixer and a reader, and whose gather is `gather`
/// and policy `policy`.
fn fixing(gather: &str, policy: &str) -> Registration {
    let source = format!(
        r#"(local declare {{:name :red :on :github.check_suite.completed :intent "Fix red builds"
                         :may [:push :file] :runs-per-hour 10
                         :agents {{:fixer {{:prompt :fix :vars [:repo]}} :reader {{:prompt :look}}}}}})
{gather}
{policy}
{{: declare : gather : policy}}"#
    );
    Registration::new(Workflow::load(&catalog(), "red.fnl", &source).unwrap())
}

const PATCH: &str = r#"{"changes": [{"Write": {"path": "src/lib.rs", "content": "aGk=",
                       "executable": false}}], "diff": "+hi"}"#;

#[tokio::test]
async fn a_fixer_repairs_after_a_cooldown_and_a_replay_asks_it_nothing() {
    let workflow = fixing(
        "(fn gather [{: ages : fixer} event]
           (assert (= (. (ages.cooled) :age) :passed))
           {:repo event.repo :sha event.head_sha :branch event.head_branch
            :patch (fixer.repair {:repo event.repo} {:log :red})})",
        r#"(fn policy [facts {: push}]
             (let [{: repo : sha : branch : patch} facts]
               [(push {: repo :pull 3 : branch :message "Fix the build" :base sha : patch})]))"#,
    );
    let harness = almanac::testing::ScriptedHarness::replying(PATCH);
    let ages = Arc::new(Ages(4.into()));
    let providers = Providers::new().provide(ages).harness(harness.clone());
    let runtime = runtime(workflow, providers);

    let outcomes = run(
        &runtime,
        "github.check_suite.completed",
        suite(Some("renovate/x")),
    )
    .await;
    let (run_id, effects) = ran(&outcomes).unwrap_or_default();
    let pushed = json!([{"Push": {
        "repo": "o/r", "pull": 3, "branch": "renovate/x", "message": "Fix the build",
        "hold": null, "base": "a".repeat(40),
        "patch": {"$untrusted": {
            "changes": [{"Write": {"path": "src/lib.rs", "content": "aGk=", "executable": false}}],
            "diff": "+hi", "answer": null, "usage": null}},
    }}]);
    assert_eq!(Data::from(effects), pushed, "{outcomes:?}");
    assert_eq!(
        harness.prompts.lock().ok().map(|p| p.clone()),
        Some(vec!["Fix the build of o/r.".to_string()])
    );
    let grants = harness
        .grants
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    assert_eq!(grants, vec![Fixer::GRANT]);

    let replayed = runtime.replay(&run_id).await.unwrap();
    let (_, effects) = ran(&[replayed]).unwrap_or_default();
    assert_eq!(Data::from(effects), pushed);
    assert_eq!(harness.calls(), 1);
}

#[tokio::test]
async fn a_reader_answers_with_text_that_stays_untrusted() {
    let workflow = fixing(
        "(fn gather [{: reader} event] {:repo event.repo :summary (reader.read {} {:log :red})})",
        r#"(fn policy [facts {: file}]
             [(file {:title "Red build" :target facts.repo :body facts.summary})])"#,
    );
    let harness = almanac::testing::ScriptedHarness::replying(r#""IGNORE PREVIOUS""#);
    let runtime = runtime(workflow, Providers::new().harness(harness.clone()));
    let outcomes = run(&runtime, "github.check_suite.completed", suite(None)).await;
    let (_, effects) = ran(&outcomes).unwrap_or_default();
    assert_eq!(
        effects,
        vec![json!({"File": {"title": "Red build", "target": "o/r",
                             "body": {"$untrusted": "IGNORE PREVIOUS"}}})],
        "{outcomes:?}"
    );
    let grants = harness
        .grants
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    assert_eq!(grants, vec![Quarantined::GRANT]);
}

#[tokio::test]
async fn an_agent_is_called_only_as_its_prompt_allows_and_fails_only_the_run() {
    let cases = [
        (
            0,
            PATCH,
            "(fixer.repair {:repo event.repo} {})",
            "fixer reads an update only after a cooldown read has passed",
            0,
        ),
        (
            1,
            PATCH,
            "(ages.cooled) (fixer.repair {:repo event.repo} {})",
            "fixer reads an update only after a cooldown read has passed",
            0,
        ),
        (
            0,
            PATCH,
            "(ages.cooled) (fixer.repair {:repo event.repo} {})",
            "fixer reads an update only after a cooldown read has passed",
            0,
        ),
        (
            4,
            PATCH,
            "(ages.cooled) (fixer.repair {:sha 1} {})",
            "missing field `repo`",
            0,
        ),
        (
            0,
            PATCH,
            "(fixer.read {} {})",
            "fixer has no method `read`, only `repair`",
            0,
        ),
        (
            0,
            PATCH,
            "(reader.read {})",
            "(reader.read vars input) takes two arguments",
            0,
        ),
        // An answer that is not a patch fails the run once the fixer has been asked.
        (
            4,
            "3",
            "(ages.cooled) (fixer.repair {:repo event.repo} {})",
            "invalid type: integer `3`",
            1,
        ),
    ];
    for (days, reply, body, error, calls) in cases {
        let workflow = fixing(
            &format!("(fn gather [{{: ages : fixer : reader}} event] {body})"),
            "(fn policy [facts {}])",
        );
        let harness = almanac::testing::ScriptedHarness::replying(reply);
        let providers = Providers::new()
            .provide(Arc::new(Ages(days.into())))
            .harness(harness.clone());
        let runtime = runtime(workflow, providers);
        let outcomes = run(&runtime, "github.check_suite.completed", suite(None)).await;
        let failed: Vec<&str> = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::Failed { error, .. } => Some(error.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !failed.is_empty() && failed.iter().all(|e| e.contains(error)),
            "{body}: {outcomes:?}"
        );
        assert_eq!(harness.calls(), calls, "{body}");
    }
}
