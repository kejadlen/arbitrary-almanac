use super::testing::{Ages, catalog};
use super::*;
use almanac::effect::EffectKind;
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

/// The run id of the one run in `outcomes`, if it ran and decided nothing.
fn ran_quietly(outcomes: &[Outcome]) -> Option<String> {
    match outcomes {
        [
            Outcome::Ran {
                run_id, effects, ..
            },
        ] if effects.is_empty() => Some(run_id.clone()),
        _ => None,
    }
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
                 :may [:attest :push] :runs-per-hour 10 :agents {:fixer {:prompt :fixer}}})
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
        "Catalog { events: [github.check_suite.completed, e], reads: [github.pull, ages.age] }"
    );
}

#[tokio::test]
async fn a_workflow_runs_only_on_its_event() {
    let workflow = on(":e", "(fn gather [{} event] (error :ran))");
    let runtime = runtime(workflow, Providers::new());
    let outcomes = run(&runtime, "github.check_suite.completed", suite(None)).await;
    assert!(outcomes.is_empty(), "{outcomes:?}");
}
