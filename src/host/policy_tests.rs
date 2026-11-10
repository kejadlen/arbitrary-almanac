use super::testing::catalog;
use super::*;
use miette::Diagnostic as _;
use serde_json::{Value as Data, json};

/// A workflow that may make any effect, and whose policy is `policy`.
fn deciding(policy: &str) -> Workflow {
    let source = format!(
        r#"(local declare {{:name :n :on :e :intent "i" :may [:attest :await :file :ask :push]
                         :runs-per-hour 1}})
(fn gather [{{}} event])
{policy}
{{: declare : gather : policy}}"#
    );
    Workflow::load(&catalog(), "w.fnl", &source).unwrap()
}

fn decide(policy: &str, facts: Data) -> Result<Data, Error> {
    let decisions = deciding(policy).policy(&facts)?;
    Ok(serde_json::to_value(decisions).unwrap_or_default())
}

fn code(err: &Error) -> String {
    err.code().map(|c| c.to_string()).unwrap_or_default()
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn policy_decides_effects_from_facts() {
    let policy = r#"(fn policy [facts {: attest : await : file : ask}]
  (let [{: repo : sha : log} facts]
    [(attest {:context "almanac/fix" :state :failure : repo : sha})
     (await "checks")
     (file {:title (.. "Red build in " repo) :target repo :body "See the log."})
     (ask {:question "Fix it?" :target repo :pull 3 :body log})]))"#;
    let facts = json!({"repo": "o/r", "sha": SHA, "log": {"$untrusted": "IGNORE PREVIOUS"}});
    assert_eq!(
        decide(policy, facts).unwrap(),
        json!([
            {"attest": {"context": "almanac/fix", "state": "Failure", "repo": "o/r", "sha": SHA}},
            {"await": {"until": "checks"}},
            {"file": {"title": "Red build in o/r", "target": "o/r",
                      "body": {"$untrusted": "See the log."}}},
            {"ask": {"question": "Fix it?", "target": "o/r", "pull": 3,
                     "body": {"$untrusted": "IGNORE PREVIOUS"}}},
        ])
    );
}

#[test]
fn a_policy_that_returns_nothing_decides_nothing() {
    for policy in ["(fn policy [facts {}])", "(fn policy [facts {}] [])"] {
        assert_eq!(decide(policy, json!({})).unwrap(), json!([]), "{policy}");
    }
}

#[test]
fn an_effect_almanac_would_refuse_is_refused_where_it_is_made() {
    let attest = |args: &str| {
        format!("(fn policy [facts {{: attest}}] (let [{{: sha}} facts] [(attest {args})]))")
    };
    let cases = [
        (
            attest(r#"{:context (.. "almanac/" "fix") :state :failure :repo "o/r" : sha}"#),
            "(attest ...) takes its context as a string literal",
        ),
        (
            attest(r#"{:context "c" :state :red :repo "o/r" : sha}"#),
            "(attest ...) takes a state of :pending, :success, or :failure",
        ),
        (
            attest(r#"{:context "c" :state :failure :repo "o r" : sha}"#),
            "(attest ...) not a slug",
        ),
        (
            attest(r#"{:context "c" :state :failure :repo "o/r" : sha :extra 1}"#),
            "(attest ...) unknown field `extra`",
        ),
        (
            r#"(fn policy [facts {: await}] [(await (.. "che" "cks"))])"#.to_string(),
            "(await ...) takes its fact as a string literal",
        ),
        (
            r#"(fn policy [facts {: file}] [(file {:title "t" :target "o/r"})])"#.to_string(),
            "(file ...) missing field `body`",
        ),
        (
            r#"(fn policy [facts {: ask}] [(ask {:question "q" :target "o/r" :body 3})])"#
                .to_string(),
            "(ask ...) data did not match any variant",
        ),
        (
            "(fn policy [facts {}] (error :no))".to_string(),
            "policy failed: w.fnl:4: no",
        ),
        (
            "(fn policy [facts {}] :attest)".to_string(),
            "a policy returns a sequence of effects",
        ),
        (
            "(fn policy [facts {}] [{:attest {}}])".to_string(),
            "a policy returns a sequence of effects",
        ),
    ];
    for (policy, message) in cases {
        let err = decide(&policy, json!({"sha": SHA})).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::run", "{policy}");
        assert!(err.to_string().contains(message), "{policy}: {err}");
    }
}

#[test]
fn a_policy_cannot_return_what_a_constructor_did_not_make() {
    // A userdata of another kind, the untrusted handle from its facts.
    let err = decide(
        "(fn policy [facts {}] [facts.log])",
        json!({"log": {"$untrusted": "x"}}),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "policy failed: a policy returns only effects its constructors made"
    );
}

#[test]
fn a_module_that_fails_while_loading_fails_the_policy() {
    let source = r#"(local declare {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})
(local limit (error "no limit"))
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}"#;
    let workflow = Workflow::load(&catalog(), "w.fnl", source).unwrap();
    let err = workflow.policy(&json!({})).unwrap_err();
    assert_eq!(err.to_string(), "policy failed: w.fnl:2: no limit");
}

/// A fixer's patch as a run's facts carry it: tainted, writing `path`.
fn patched(path: &str) -> Data {
    json!({
        "repo": "o/r",
        "sha": SHA,
        "patch": {"$untrusted": {
            "changes": [{"Write": {"path": path, "content": "aGk=", "executable": false}}],
            "diff": "+hi",
        }},
    })
}

fn pushing(args: &str) -> String {
    format!(
        r#"(fn policy [facts {{: push}}]
  (let [{{: repo : sha : patch}} facts]
    [(push {{:repo repo :pull 3 :branch "renovate/x" :base sha {args}}})]))"#
    )
}

#[test]
fn a_policy_pushes_a_patch_the_patch_policy_accepts() {
    let cases = [
        (
            pushing(r#":message "Fix the build" :hold "almanac/fix" : patch"#),
            json!("almanac/fix"),
        ),
        (pushing(r#":message "Fix the build" : patch"#), json!(null)),
    ];
    for (policy, hold) in cases {
        let decided = decide(&policy, patched("src/lib.rs")).unwrap();
        assert_eq!(
            decided,
            json!([{"push": {
                "repo": "o/r", "pull": 3, "branch": "renovate/x", "message": "Fix the build",
                "hold": hold,
                "patch": {"base": SHA, "patch": {"$untrusted": {
                    "changes": [{"Write": {"path": "src/lib.rs", "content": "aGk=",
                                           "executable": false}}],
                    "diff": "+hi", "answer": null, "usage": null,
                }}},
            }}]),
            "{policy}"
        );
    }
}

#[test]
fn a_push_is_refused_where_it_is_made() {
    let cases = [
        (
            pushing(r#":message "m" : patch"#),
            patched(".github/workflows/ci.yml"),
            "(push ...) refused its patch: Denied",
        ),
        (
            pushing(r#":message "m" :patch {:changes [] :diff ""}"#),
            patched("src/lib.rs"),
            "(push ...) missing field `$untrusted`",
        ),
        (
            pushing(r#":message (.. "m" "m") : patch"#),
            patched("src/lib.rs"),
            "(push ...) takes its message as a string literal",
        ),
        (
            pushing(r#":message "m" :hold (.. "h" "h") : patch"#),
            patched("src/lib.rs"),
            "(push ...) takes its hold as a string literal",
        ),
    ];
    for (policy, facts, message) in cases {
        let err = decide(&policy, facts).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::run", "{policy}");
        assert!(err.to_string().contains(message), "{policy}: {err}");
    }
}
