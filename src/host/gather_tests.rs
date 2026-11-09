use super::*;
use crate::Catalog;
use miette::Diagnostic as _;
use serde_json::json;

type Answer = fn(&Call) -> Result<Data, String>;

/// Answers every call with what `answer` says, and keeps the calls.
struct Fake {
    calls: Vec<Call>,
    answer: Answer,
}

impl Fake {
    fn answering(answer: Answer) -> Self {
        Self {
            calls: vec![],
            answer,
        }
    }
}

impl Dispatch for Fake {
    fn call(&mut self, call: Call) -> impl Future<Output = Result<Data, String>> + Send {
        let answer = (self.answer)(&call);
        self.calls.push(call);
        async move { answer }
    }
}

fn nothing(_: &Call) -> Result<Data, String> {
    Ok(Data::Null)
}

/// A workflow whose gather is `gather`.
fn gathering(gather: &str) -> String {
    format!(
        r#"(local declare {{:name :n :on :e :intent "i" :may [] :runs-per-hour 1
                         :agents {{:fixer {{:prompt :fixer}}}}}})
{gather}
(fn policy [facts {{}}])
{{: declare : gather : policy}}"#
    )
}

fn load(source: &str) -> Result<Workflow, Error> {
    let catalog = Catalog {
        providers: vec!["github", "ages"],
    };
    Workflow::load(&catalog, "w.fnl", source)
}

async fn gather(body: &str, dispatch: &mut Fake) -> Result<Data, Error> {
    let source = gathering(&format!(
        "(fn gather [{{: github : fixer}} event]\n  {body})"
    ));
    load(&source)?
        .gather(&json!({"repo": "o/r"}), dispatch)
        .await
}

fn code(err: &Error) -> String {
    err.code().map(|c| c.to_string()).unwrap_or_default()
}

#[tokio::test]
async fn gather_reads_through_its_capabilities_and_returns_facts() {
    let mut fake = Fake::answering(|call| match call.method.as_str() {
        "pull" => Ok(json!({"head": "abc", "found": true})),
        "checks" => Ok(json!([{"x": 2}, {"x": 3}])),
        _ => Ok(json!("patched")),
    });
    let body = r#"
  (let [pull (github.pull event.repo 3)
        total (accumulate [n 0 _ check (ipairs (github.checks pull.head))] (+ n check.x))]
    (case pull
      {:found true} {:kind :found : total :patch (fixer.repair pull.head)}
      _ {:kind :nothing}))"#;
    let facts = gather(body, &mut fake).await.unwrap();

    assert_eq!(
        facts,
        json!({"kind": "found", "total": 5, "patch": "patched"})
    );
    let calls: Vec<(&str, &str, &[Data])> = fake
        .calls
        .iter()
        .map(|c| (c.capability.as_str(), c.method.as_str(), c.args.as_slice()))
        .collect();
    assert_eq!(
        calls,
        [
            ("github", "pull", &[json!("o/r"), json!(3)][..]),
            ("github", "checks", &[json!("abc")][..]),
            ("fixer", "repair", &[json!("abc")][..]),
        ]
    );
}

#[tokio::test]
async fn gather_with_nothing_to_say_returns_null() {
    let facts = gather("(github.pull)", &mut Fake::answering(nothing))
        .await
        .unwrap();
    assert_eq!(facts, Data::Null);
}

#[test]
fn a_workflow_can_name_only_its_locals_and_pure_globals() {
    for name in [
        "os.exit",
        "ages",
        "require",
        "print",
        "tostring",
        "load",
        "coroutine",
        "next",
    ] {
        let source = gathering(&format!("(fn gather [{{: github}} event] ({name}))"));
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::compile", "{name}");
        assert!(err.to_string().contains("unknown identifier"), "{err}");
        assert!(err.labels().is_some(), "{name}");
    }
}

#[test]
fn nothing_may_run_while_compiling() {
    let cases = [
        (
            r#"(fn gather [{: github} event] (lua "return os"))"#,
            "lua",
            r#"(lua "return os")"#,
        ),
        (
            "(fn gather [{: github} event] (eval-compiler (print :hi)))",
            "eval-compiler",
            "(eval-compiler (print :hi))",
        ),
        (
            "(fn gather [{: github} event] [(macro m [] 1)])",
            "macro",
            "(macro m [] 1)",
        ),
        (
            "(fn gather [{: github} event] {:x (include :other)})",
            "include",
            "(include :other)",
        ),
    ];
    for (gather, form, label) in cases {
        let source = gathering(gather);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::compile_time", "{gather}");
        assert_eq!(
            err.to_string(),
            format!("`{form}` runs code while compiling, so a workflow may not use it")
        );
        let at = err.labels().unwrap().next().unwrap();
        assert_eq!(&source[at.offset()..at.offset() + at.len()], label);
    }
}

#[tokio::test]
async fn pairs_walks_numbers_then_strings_in_order() {
    let body = r#"(icollect [k _ (pairs {:b 1 :a 2 10 3 2 4 1.5 5})] k)"#;
    let facts = gather(body, &mut Fake::answering(nothing)).await.unwrap();
    assert_eq!(facts, json!([1.5, 2, 10, "a", "b"]));

    let err = gather(
        "(each [k (pairs {true 1})] k)",
        &mut Fake::answering(nothing),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("pairs walks only tables keyed by numbers and strings, not a boolean"),
        "{err}"
    );
}

#[tokio::test]
async fn the_sandbox_holds_nothing_impure() {
    let body = r#"{:os (. _G :os) :random (. math :random) :dump (. string :dump)
                   :floor (math.floor 2.5)}"#;
    let facts = gather(body, &mut Fake::answering(nothing)).await.unwrap();
    assert_eq!(facts, json!({"floor": 2}));
}

#[tokio::test]
async fn a_run_may_work_hard_within_its_budget() {
    let body = "(faccumulate [n 0 i 1 100000] (+ n i))";
    let facts = gather(body, &mut Fake::answering(nothing)).await.unwrap();
    assert_eq!(facts, json!(5000050000_i64));
}

#[tokio::test]
async fn a_run_stops_after_its_budget() {
    let err = gather("(while true nil)", &mut Fake::answering(nothing))
        .await
        .unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::run");
    assert!(err.to_string().contains("instruction budget"), "{err}");
}

#[tokio::test]
async fn failures_inside_gather_end_the_run() {
    let cases: [(&str, Answer, &str); 4] = [
        ("(error \"boom\")", nothing, "gather failed: w.fnl:4: boom"),
        (
            "(github.pull)",
            |_| Err("GitHub is down".into()),
            "gather failed: GitHub is down",
        ),
        ("(: github :pull)", nothing, "gather failed:"),
        ("gather", nothing, "gather failed:"),
    ];
    for (body, answer, message) in cases {
        let err = gather(body, &mut Fake::answering(answer))
            .await
            .unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::run", "{body}");
        assert!(err.to_string().starts_with(message), "{body}: {err}");
    }
}

#[test]
fn a_run_loads_only_text() {
    let sandbox = Sandbox::new().unwrap();
    let err = sandbox.exports("w.lua", "\x1bLua").unwrap_err();
    assert!(err.to_string().contains("binary"), "{err}");
}

#[test]
fn a_gather_can_run_on_any_thread() {
    fn send<T: Send>(_: T) {}
    let workflow = load(&gathering("(fn gather [{: github} event])")).unwrap();
    let event = json!({});
    send(workflow.gather(&event, &mut Fake::answering(nothing)));
}
