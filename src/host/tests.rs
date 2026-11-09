use super::*;
use almanac::effect::EffectKind;
use miette::{Diagnostic as _, SourceSpan};

/// The smallest declaration a workflow can make.
const MINIMAL: &str = r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1}"#;

fn load(source: &str) -> Result<Workflow, Error> {
    Workflow::load("w.fnl", source)
}

/// A workflow that declares `declare`, with a gather and a policy that ask for nothing.
fn declaring(declare: &str) -> String {
    ending_with(declare, "{: declare : gather : policy}")
}

/// A workflow that declares `declare` and whose last form is `last`.
fn ending_with(declare: &str, last: &str) -> String {
    format!("(local declare {declare})\n(fn gather [{{}} event])\n(fn policy [facts {{}}])\n{last}")
}

fn code(err: &Error) -> String {
    err.code().map(|c| c.to_string()).unwrap_or_default()
}

/// Where an error's label points, as the text under it.
fn labelled(err: &Error, source: &str) -> Option<String> {
    let label = err.labels()?.next()?;
    source
        .get(label.offset()..label.offset() + label.len())
        .map(str::to_string)
}

#[test]
fn reads_a_declaration_without_running_anything() {
    let source = r#"
(local declare
  {:name :build-failures
   :on :github.check_suite.completed
   :intent "Repair a Renovate update whose build fails"
   :may [:attest :ask :push]
   :runs-per-hour 30
   :agents {:fixer {:prompt :fixer :vars [:repo]}}})

(fn gather [{} event] (error "gather must not run at load"))

(fn policy [facts {}] (error "policy must not run at load"))

(error "the top level must not run at load")

{: declare : gather : policy}
"#;
    let workflow = load(source).unwrap();

    assert_eq!(
        workflow.declaration(),
        &Declaration {
            name: "build-failures".into(),
            on: "github.check_suite.completed".into(),
            intent: "Repair a Renovate update whose build fails".into(),
            may: vec![EffectKind::Attest, EffectKind::Ask, EffectKind::Push],
            runs_per_hour: 30,
            agents: [(
                "fixer".to_string(),
                Agent {
                    prompt: "fixer".into(),
                    vars: vec!["repo".into()],
                },
            )]
            .into(),
        }
    );
}

#[test]
fn agents_and_their_vars_may_be_left_out() {
    let workflow = load(&declaring(MINIMAL)).unwrap();
    assert!(workflow.declaration().agents.is_empty());

    let source = declaring(
        r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1
            :agents {:fixer {:prompt :fixer}}}"#,
    );
    let workflow = load(&source).unwrap();
    assert!(workflow.declaration().agents["fixer"].vars.is_empty());
}

#[test]
fn a_parse_error_points_at_where_it_happened() {
    let source = "{: declare\n : gather";
    let err = load(source).unwrap_err();

    assert_eq!(code(&err), "arbitrary_almanac::parse");
    assert!(err.to_string().starts_with("w.fnl:2:"), "{err}");
    assert!(err.to_string().contains("Parse error"), "{err}");
    let label = err.labels().unwrap().next().unwrap();
    assert_eq!(label.offset(), source.len());
}

#[test]
fn the_offset_of_an_error_is_found_on_its_line() {
    let source = Source::new("w.fnl", "ab\ncdef\n");

    let at = |message: &str| source.offset_of(message).map(|s| s.offset());
    assert_eq!(at("w.fnl:1:1: Parse error"), Some(1));
    assert_eq!(at("w.fnl:2:2: Parse error"), Some(5));
    assert_eq!(at("w.fnl:9:9: Parse error"), Some(8));
    assert_eq!(at("x.fnl:1:1: Parse error"), None);
    assert_eq!(at("w.fnl;1:1: Parse error"), None);
    assert_eq!(at("w.fnl:a:1: Parse error"), None);
    assert_eq!(at("w.fnl:1:a: Parse error"), None);
    assert_eq!(at("w.fnl:1"), None);
}

#[test]
fn a_workflow_must_end_with_its_exports() {
    let refused = [
        "(local declare {})",
        "[declare gather policy]",
        "{: declare : gather}",
        "{: declare : gather : policy :extra 1}",
        "{: declare : gather :policy gather}",
        "{: declare : gather :policy :policy}",
    ];
    for last in refused {
        let source = ending_with(MINIMAL, last);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::exports", "{last}");
        assert_eq!(labelled(&err, &source).as_deref(), Some(last));
    }
}

#[test]
fn an_export_error_points_at_the_last_form_when_it_can() {
    let source = ending_with(MINIMAL, ":done");
    let err = load(&source).unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::exports");
    assert_eq!(labelled(&err, &source), None);

    let source = "(local declare {})\n(fn policy [facts {}])";
    let err = load(source).unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::exports");
    assert_eq!(
        labelled(&err, source).as_deref(),
        Some("(fn policy [facts {}])")
    );

    let err = load("").unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::exports");
    assert_eq!(labelled(&err, ""), None);
}

#[test]
fn a_workflow_declares_itself_exactly_once() {
    let none = r#"
(local declared {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}
"#;
    let twice = r#"
(local declare {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})
(local declare {:name :m :on :e :intent "i" :may [] :runs-per-hour 1})
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}
"#;
    for source in [none, twice] {
        let err = load(source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::declare");
    }
}

#[test]
fn other_top_level_forms_are_not_mistaken_for_the_declaration() {
    let source = r#"
(local declare {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})
(var declare {})
(local)
:stray
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}
"#;
    assert!(load(source).is_ok());
}

#[test]
fn a_declaration_that_names_or_calls_anything_is_refused() {
    let cases = [
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour per-hour}"#,
            "per-hour",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour (* 3 10)}"#,
            "(* 3 10)",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [attest] :runs-per-hour 1}"#,
            "attest",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [:attest nil] :runs-per-hour 1}"#,
            "nil",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [...] :runs-per-hour 1}"#,
            "...",
        ),
        (
            r#"{name :n :on :e :intent "i" :may [] :runs-per-hour 1}"#,
            "name",
        ),
        (
            r#"(identity {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})"#,
            r#"(identity {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})"#,
        ),
    ];
    for (declare, label) in cases {
        let source = declaring(declare);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::literal", "{declare}");
        assert_eq!(labelled(&err, &source).as_deref(), Some(label), "{declare}");
    }
}

#[test]
fn a_declaration_must_have_the_right_shape() {
    let cases = [
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1 :typo 1}"#,
            "unknown field `typo`",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may []}"#,
            "missing field `runs-per-hour`",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [:merge] :runs-per-hour 1}"#,
            "unknown variant `Merge`",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1
                :agents {:fixer {:prompt :fixer :model :opus}}}"#,
            "unknown field `model`",
        ),
    ];
    for (declare, message) in cases {
        let source = declaring(declare);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::declaration", "{declare}");
        assert!(err.to_string().contains(message), "{err}");
        assert_eq!(labelled(&err, &source).as_deref(), Some(declare));
    }
}

#[test]
fn a_declaration_must_be_a_table() {
    let source = declaring(":oops");
    let err = load(&source).unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::declaration");
    assert_eq!(labelled(&err, &source), None);
}

#[test]
fn spans_need_both_ends() {
    let lua = mlua::Lua::new();
    let fennel = lua
        .load("return {['ast-source'] = function(t) return getmetatable(t) end}")
        .eval::<mlua::Table>()
        .unwrap();
    let table = |pairs: &[(&str, i64)]| {
        let t = lua.create_table().unwrap();
        let mt = lua.create_table().unwrap();
        for (k, v) in pairs {
            mt.set(*k, *v).unwrap();
        }
        t.set_metatable(Some(mt)).unwrap();
        t
    };
    let span = |pairs: &[(&str, i64)]| form::span(&fennel, &table(pairs));

    assert_eq!(form::span(&lua.create_table().unwrap(), &table(&[])), None);
    assert_eq!(form::span(&fennel, &lua.create_table().unwrap()), None);
    assert_eq!(span(&[("byteend", 3)]), None);
    assert_eq!(span(&[("bytestart", 1)]), None);
    assert_eq!(span(&[("bytestart", 5), ("byteend", 3)]), None);
    assert_eq!(span(&[("bytestart", 0), ("byteend", 3)]), None);
    assert_eq!(
        span(&[("bytestart", 2), ("byteend", 3)]),
        Some(SourceSpan::from((1, 2)))
    );
}

#[test]
fn only_what_the_parser_makes_becomes_a_form() {
    let lua = mlua::Lua::new();
    let err = lua.unpack::<Form>(mlua::Value::Nil).unwrap_err();
    assert!(err.to_string().contains("Form"), "{err}");
}

#[test]
fn forms_read_as_data() {
    let source = r#"
(local declare {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})
(local other {1 :one [:k] :seq 2.5 :float false :no})
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}
"#;
    load(source).unwrap();
    let forms = Fennel::new()
        .unwrap()
        .parse(&Source::new("w.fnl", source))
        .unwrap();
    let other = local_named(&forms[1], "other").unwrap();
    let data = other.to_data().unwrap();

    assert_eq!(
        data,
        serde_json::json!({"1": "one", "[\"k\"]": "seq", "2.5": "float", "false": "no"})
    );
    assert_eq!(Form::Str("x".into()).span(), None);
    assert_eq!(Form::Bool(true).symbol(), None);
}
