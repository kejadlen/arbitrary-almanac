use super::*;
use crate::Catalog;
use almanac::effect::EffectKind;
use hegel::TestCase;
use hegel::generators::{self as gs, Generator as _};
use miette::Diagnostic as _;
use std::collections::BTreeMap;

/// The smallest declaration a workflow can make.
const MINIMAL: &str = r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1}"#;

fn load(source: &str) -> Result<Workflow, Error> {
    Workflow::load(&catalog(), "w.fnl", source)
}

fn catalog() -> Catalog {
    Catalog {
        providers: vec!["github", "ages"],
    }
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
fn other_locals_are_not_mistaken_for_the_declaration() {
    let source = r#"
(local declare {:name :n :on :e :intent "i" :may [] :runs-per-hour 1})
(local declared {})
(fn declare-again [] {})
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}
"#;
    assert!(load(source).is_ok());
}

#[test]
fn the_top_level_holds_only_locals_and_functions() {
    let cases = [
        (":stray", None),
        ("(var x 1)", Some("(var x 1)")),
        ("(print :hello)", Some("(print :hello)")),
        ("(macro twice [x] x)", Some("(macro twice [x] x)")),
    ];
    for (form, label) in cases {
        let source = ending_with(MINIMAL, &format!("{form}\n{{: declare : gather : policy}}"));
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::top_level", "{form}");
        assert_eq!(labelled(&err, &source).as_deref(), label, "{form}");
    }
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
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1 :typo true}"#,
            "unknown field `typo`",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may []}"#,
            "missing field `runs-per-hour`",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [:merge] :runs-per-hour 1}"#,
            "invalid value: string \"merge\"",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [:Attest] :runs-per-hour 1}"#,
            "invalid value: string \"Attest\", expected an effect such as :attest",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [""] :runs-per-hour 1}"#,
            "invalid value: string \"\", expected an effect such as :attest",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may :attest :runs-per-hour 1}"#,
            "invalid type: string \"attest\", expected a sequence",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1.5}"#,
            "invalid type: floating point `1.5`, expected u32",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1 1 true}"#,
            "unknown field `1`",
        ),
        (
            r#"{:name :n :on :e :intent "i" :may [] :runs-per-hour 1 [:k] true}"#,
            "unknown field `[\"k\"]`",
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
fn every_effect_is_almanacs_own() {
    let source = declaring(
        r#"{:name :n :on :e :intent "i" :may [:attest :await :file :ask :push] :runs-per-hour 1}"#,
    );
    let workflow = load(&source).unwrap();
    assert_eq!(
        workflow.declaration().may,
        [
            EffectKind::Attest,
            EffectKind::Await,
            EffectKind::File,
            EffectKind::Ask,
            EffectKind::Push,
        ]
    );
}

/// A Fennel string literal for any text.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `declaration` written as the Fennel a workflow would declare it with.
fn fennel(declaration: &Declaration) -> String {
    let strings = |items: &[String]| {
        items
            .iter()
            .map(|s| quoted(s))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let may: Vec<String> = declaration
        .may
        .iter()
        .map(|kind| format!(":{}", format!("{kind:?}").to_lowercase()))
        .collect();
    let agents: Vec<String> = declaration
        .agents
        .iter()
        .map(|(name, agent)| {
            format!(
                "{} {{:prompt {} :vars [{}]}}",
                quoted(name),
                quoted(&agent.prompt),
                strings(&agent.vars)
            )
        })
        .collect();
    format!(
        "{{:name {} :on {} :intent {} :may [{}] :runs-per-hour {} :agents {{{}}}}}",
        quoted(&declaration.name),
        quoted(&declaration.on),
        quoted(&declaration.intent),
        may.join(" "),
        declaration.runs_per_hour,
        agents.join(" ")
    )
}

#[hegel::composite]
fn declarations(tc: &TestCase) -> Declaration {
    let text = || gs::text().max_size(12);
    let kinds = vec![
        EffectKind::Attest,
        EffectKind::Await,
        EffectKind::File,
        EffectKind::Ask,
        EffectKind::Push,
    ];
    let agent = || {
        hegel::tuples!(text(), gs::vecs(text()).max_size(3))
            .map(|(prompt, vars)| Agent { prompt, vars })
    };
    Declaration {
        name: tc.draw(text()),
        on: tc.draw(text()),
        intent: tc.draw(text()),
        may: tc.draw(gs::vecs(gs::sampled_from(kinds)).max_size(5)),
        runs_per_hour: tc.draw(gs::integers::<u32>()),
        agents: tc
            .draw(gs::vecs(hegel::tuples!(text(), agent())).max_size(3))
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
    }
}

#[hegel::test]
fn any_declaration_reads_back_as_written(tc: TestCase) {
    let declaration = tc.draw(declarations());
    let workflow = load(&declaring(&fennel(&declaration))).unwrap();
    assert_eq!(workflow.declaration(), &declaration);
}

#[hegel::test]
fn any_text_loads_or_is_refused_without_panicking(tc: TestCase) {
    let _ = load(&tc.draw(gs::text()));
}

/// A workflow that may attest and push, has a `fixer` agent, and defines `gather` and `policy`.
fn defining(gather: &str, policy: &str) -> String {
    format!(
        r#"(local declare {{:name :n :on :e :intent "i" :may [:attest :push] :runs-per-hour 1
                         :agents {{:fixer {{:prompt :fixer}}}}}})
{gather}
{policy}
{{: declare : gather : policy}}"#
    )
}

#[test]
fn capabilities_come_from_what_gather_and_policy_destructure() {
    let source = defining(
        "(fn gather [{: github : fixer :ages a} event])",
        "(fn policy [facts {: push : attest}])",
    );
    let workflow = load(&source).unwrap();

    assert_eq!(
        workflow.capabilities(),
        &Capabilities {
            providers: vec!["ages".into(), "github".into()],
            agents: vec!["fixer".into()],
            effects: vec![EffectKind::Attest, EffectKind::Push],
        }
    );
}

#[test]
fn gather_and_policy_are_each_defined_once() {
    let cases = [
        defining(
            "(fn gathered [{: github} event])",
            "(fn policy [facts {: attest}])",
        ),
        defining("(fn gather)", "(fn policy [facts {: attest}])"),
        defining(
            "(fn gather [{: github} event])\n(fn gather [{: github} event])",
            "(fn policy [facts {: attest}])",
        ),
        defining(
            "(fn gather [{: github} event])",
            "(fn policy-of [facts {: attest}])",
        ),
    ];
    for source in cases {
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::undefined", "{source}");
    }
}

#[test]
fn gather_and_policy_take_their_capabilities_as_a_table() {
    let cases = [
        (
            defining("(fn gather [cx event])", "(fn policy [facts {: attest}])"),
            "gather must take [{: provider : agent} event]",
            "(fn gather [cx event])",
        ),
        (
            defining("(fn gather [{: github}])", "(fn policy [facts {: attest}])"),
            "gather must take [{: provider : agent} event]",
            "(fn gather [{: github}])",
        ),
        (
            defining(
                "(fn gather [{: github} event])",
                "(fn policy [{: attest} facts])",
            ),
            "policy must take [facts {: effect}]",
            "(fn policy [{: attest} facts])",
        ),
        (
            defining(
                "(fn gather [{:github [a b]} event])",
                "(fn policy [facts {: attest}])",
            ),
            "a plain `: name`",
            "[a b]",
        ),
        (
            defining(
                "(fn gather [{: github} event])",
                "(fn policy [facts {1 attest}])",
            ),
            "a plain `: name`",
            "attest",
        ),
    ];
    for (source, message, label) in cases {
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::parameters", "{source}");
        assert!(err.to_string().contains(message), "{err}");
        assert_eq!(labelled(&err, &source).as_deref(), Some(label));
    }
}

#[test]
fn a_capability_must_be_something_the_workflow_may_have() {
    let cases = [
        (
            defining(
                "(fn gather [{: github : linear} event])",
                "(fn policy [facts {: attest}])",
            ),
            "gather cannot be handed `linear`: no provider or agent has that name",
        ),
        (
            defining(
                "(fn gather [{: github} event])",
                "(fn policy [facts {: ask}])",
            ),
            "policy cannot be handed `ask`: it is not in :may",
        ),
        (
            defining(
                "(fn gather [{: github} event])",
                "(fn policy [facts {: merge}])",
            ),
            "policy cannot be handed `merge`: Almanac has no such effect",
        ),
    ];
    for (source, message) in cases {
        let err = load(&source).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::capability", "{source}");
        assert_eq!(err.to_string(), message);
        let name = message.split('`').nth(1).unwrap();
        assert_eq!(labelled(&err, &source).as_deref(), Some(name));
    }
}

#[hegel::test]
fn a_workflow_is_handed_exactly_what_it_destructures(tc: TestCase) {
    let kinds = vec![
        EffectKind::Attest,
        EffectKind::Await,
        EffectKind::File,
        EffectKind::Ask,
        EffectKind::Push,
    ];
    // Ending in a digit, so that no name is one of Fennel's special forms.
    let name = || gs::from_regex("[a-z][a-z-]{0,5}[0-9]").fullmatch(true);
    let agents: BTreeMap<String, ()> = tc
        .draw(gs::vecs(name()).max_size(3))
        .into_iter()
        .map(|agent| (agent, ()))
        .collect();
    let may = tc.draw(gs::vecs(gs::sampled_from(kinds.clone())).max_size(5));
    let mut providers = tc.draw(gs::vecs(gs::sampled_from(vec!["ages", "github"])).max_size(2));
    providers.sort();
    providers.dedup();
    let asked_agents: Vec<&String> = agents.keys().filter(|_| tc.draw(gs::booleans())).collect();
    let effects: Vec<EffectKind> = kinds
        .iter()
        .copied()
        .filter(|kind| may.contains(kind) && tc.draw(gs::booleans()))
        .collect();

    let lower = |kind: &EffectKind| format!("{kind:?}").to_lowercase();
    let destructure = |names: Vec<String>| {
        let names: Vec<String> = names.iter().map(|n| format!(": {n}")).collect();
        format!("{{{}}}", names.join(" "))
    };
    let gather_names = providers
        .iter()
        .map(|p| p.to_string())
        .chain(asked_agents.iter().map(|a| a.to_string()))
        .collect();
    let source = format!(
        "(local declare {{:name :n :on :e :intent \"i\" :may [{}] :runs-per-hour 1 :agents {{{}}}}})
(fn gather [{} event])
(fn policy [facts {}])
{{: declare : gather : policy}}",
        may.iter().map(|k| format!(":{}", lower(k))).collect::<Vec<_>>().join(" "),
        agents.keys().map(|a| format!("{} {{:prompt :p}}", quoted(a))).collect::<Vec<_>>().join(" "),
        destructure(gather_names),
        destructure(effects.iter().map(lower).collect()),
    );
    let workflow = load(&source).unwrap();

    let in_may_order: Vec<EffectKind> = may
        .iter()
        .copied()
        .filter(|kind| effects.contains(kind))
        .fold(vec![], |mut seen, kind| {
            if !seen.contains(&kind) {
                seen.push(kind);
            }
            seen
        });
    assert_eq!(
        workflow.capabilities(),
        &Capabilities {
            providers: providers.iter().map(|p| p.to_string()).collect(),
            agents: asked_agents.into_iter().cloned().collect(),
            effects: in_may_order,
        }
    );
}
