use super::*;
use almanac::effect::EffectKind;
use miette::{Diagnostic as _, SourceSpan};

const WORKFLOW: &str = r#"
(local declare
  {:name :build-failures
   :on :github.check_suite.completed
   :intent "Repair a Renovate update whose build fails"
   :may [:attest :ask :push]
   :runs-per-hour 30
   :agents {:fixer {:prompt :fixer :vars [:repo]}}})

(fn gather [{: github} event] (error "gather must not run at load"))

(fn policy [facts {: attest}] [])

(error "the top level must not run at load")

{: declare : gather : policy}
"#;

fn load(source: &str) -> Result<Workflow, Error> {
    Workflow::load("w.fnl", source)
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

fn replace(from: &str, to: &str) -> String {
    WORKFLOW.replace(from, to)
}

#[test]
fn reads_a_declaration_without_running_anything() {
    let workflow = load(WORKFLOW).unwrap();

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
    let source = replace(":agents {:fixer {:prompt :fixer :vars [:repo]}}", "");
    assert!(load(&source).unwrap().declaration().agents.is_empty());

    let source = replace(" :vars [:repo]", "");
    let workflow = load(&source).unwrap();
    assert_eq!(
        workflow.declaration().agents["fixer"].vars,
        Vec::<String>::new()
    );
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
        ("", Some("(error \"the top level must not run at load\")")),
        ("(local declare {})", Some("(local declare {})")),
        ("[declare gather policy]", Some("[declare gather policy]")),
        ("{: declare : gather}", Some("{: declare : gather}")),
        (
            "{: declare : gather : policy :extra 1}",
            Some("{: declare : gather : policy :extra 1}"),
        ),
        (
            "{: declare : gather :policy gather}",
            Some("{: declare : gather :policy gather}"),
        ),
        (
            "{: declare : gather :policy :policy}",
            Some("{: declare : gather :policy :policy}"),
        ),
        (":done", None),
    ];
    for (last, label) in refused {
        let source = replace("{: declare : gather : policy}", last);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::exports", "{last}");
        assert_eq!(labelled(&err, &source).as_deref(), label, "{last}");
    }
}

#[test]
fn an_empty_file_has_no_exports() {
    let err = load("").unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::exports");
    assert_eq!(labelled(&err, ""), None);
}

#[test]
fn a_workflow_declares_itself_exactly_once() {
    let none = replace("(local declare", "(local declared");
    let twice = replace("(fn policy", "(local declare {})\n(fn policy");
    for source in [none, twice] {
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::declare");
    }
}

#[test]
fn other_top_level_forms_are_not_mistaken_for_the_declaration() {
    let source = replace(
        "(fn policy",
        "(var declare {})\n(local)\n:stray\n(fn policy",
    );
    assert!(load(&source).is_ok());
}

#[test]
fn a_declaration_that_names_or_calls_anything_is_refused() {
    let cases = [
        (":runs-per-hour 30", ":runs-per-hour per-hour", "per-hour"),
        (":runs-per-hour 30", ":runs-per-hour (* 3 10)", "(* 3 10)"),
        (":vars [:repo]", ":vars [repo]", "repo"),
        (":vars [:repo]", ":vars [:repo nil]", "nil"),
        (":vars [:repo]", ":vars [...]", "..."),
        (":fixer {", "fixer {", "fixer"),
    ];
    for (from, to, label) in cases {
        let source = replace(from, to);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::literal", "{to}");
        assert_eq!(labelled(&err, &source).as_deref(), Some(label), "{to}");
    }

    let source = replace("(local declare\n  {", "(local declare (identity {");
    let source = source.replacen("[:repo]}}})", "[:repo]}}}))", 1);
    let err = load(&source).unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::literal");
}

#[test]
fn a_declaration_must_have_the_right_shape() {
    let cases = [
        (
            ":runs-per-hour 30",
            ":runs-per-hour 30 :typo 1",
            "unknown field `typo`",
        ),
        (":runs-per-hour 30", "", "missing field `runs-per-hour`"),
        (
            ":may [:attest :ask :push]",
            ":may [:merge]",
            "unknown variant `Merge`",
        ),
        (
            ":vars [:repo]",
            ":vars [:repo] :model :opus",
            "unknown field `model`",
        ),
    ];
    for (from, to, message) in cases {
        let source = replace(from, to);
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::declaration", "{to}");
        assert!(err.to_string().contains(message), "{err}");
        let label = labelled(&err, &source).unwrap();
        assert!(label.starts_with("{:name :build-failures"), "{label}");
    }
}

#[test]
fn a_declaration_may_be_a_bare_literal() {
    let source = replace(
        "(local declare\n  {",
        "(local declare :oops)\n(local unused\n  {",
    );
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
    let source = r#"(local declare {:name :x :on :e :intent "i" :may [] :runs-per-hour 1
                                    :agents {}})
                    (local other {1 :one [:k] :seq 2.5 :float false :no})
                    {: declare : gather : policy}"#;
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
