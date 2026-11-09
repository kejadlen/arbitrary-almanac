use miette::{Diagnostic, NamedSource, SourceSpan};
use std::sync::Arc;

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("io error: {0}")]
    #[diagnostic(code(arbitrary_almanac::io))]
    Io(#[from] std::io::Error),

    /// The Fennel host itself failed, not a workflow.
    #[error("lua error: {0}")]
    #[diagnostic(code(arbitrary_almanac::lua))]
    Lua(#[from] mlua::Error),

    #[error("{message}")]
    #[diagnostic(code(arbitrary_almanac::parse))]
    Parse {
        message: String,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("here")]
        at: Option<SourceSpan>,
    },

    #[error("no event in this binary is called `{on}`")]
    #[diagnostic(
        code(arbitrary_almanac::event),
        help("`:on` names an event from the catalog, such as :github.check_suite.completed")
    )]
    Event {
        on: String,
        #[source_code]
        src: Arc<NamedSource<String>>,
    },

    #[error("{message}")]
    #[diagnostic(code(arbitrary_almanac::compile))]
    Compile {
        message: String,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("here")]
        at: Option<SourceSpan>,
    },

    #[error(
        "only (local ...) and (fn ...) may come before the exports, because the top level runs on every run"
    )]
    #[diagnostic(code(arbitrary_almanac::top_level))]
    TopLevel {
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("not a local or a function")]
        at: Option<SourceSpan>,
    },

    #[error("`{form}` runs code while compiling, so a workflow may not use it")]
    #[diagnostic(code(arbitrary_almanac::compile_time))]
    CompileTime {
        form: String,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("here")]
        at: Option<SourceSpan>,
    },

    /// A run of a workflow's function failed.
    #[error("{function} failed: {message}")]
    #[diagnostic(code(arbitrary_almanac::run))]
    Run {
        function: &'static str,
        message: String,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("here")]
        at: Option<SourceSpan>,
    },

    #[error("a workflow must end with {{: declare : gather : policy}}")]
    #[diagnostic(code(arbitrary_almanac::exports))]
    Exports {
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("the last form")]
        at: Option<SourceSpan>,
    },

    #[error("a workflow must declare itself once, with (local declare {{...}})")]
    #[diagnostic(code(arbitrary_almanac::declare))]
    Undeclared {
        #[source_code]
        src: Arc<NamedSource<String>>,
    },

    #[error("a declaration is read without running it, so it must be a literal")]
    #[diagnostic(code(arbitrary_almanac::literal))]
    NotLiteral {
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("not a literal")]
        at: Option<SourceSpan>,
    },

    #[error("invalid declaration: {message}")]
    #[diagnostic(code(arbitrary_almanac::declaration))]
    Declaration {
        message: String,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("this declaration")]
        at: Option<SourceSpan>,
    },

    #[error("a workflow must define (fn {function} [...] ...) once, at the top level")]
    #[diagnostic(code(arbitrary_almanac::undefined))]
    Undefined {
        function: &'static str,
        #[source_code]
        src: Arc<NamedSource<String>>,
    },

    #[error("{function} must take {expected}")]
    #[diagnostic(code(arbitrary_almanac::parameters))]
    Parameters {
        function: &'static str,
        expected: &'static str,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("here")]
        at: Option<SourceSpan>,
    },

    #[error("{function} cannot be handed `{name}`: {why}")]
    #[diagnostic(code(arbitrary_almanac::capability))]
    Capability {
        function: &'static str,
        name: String,
        why: &'static str,
        #[source_code]
        src: Arc<NamedSource<String>>,
        #[label("asked for here")]
        at: Option<SourceSpan>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_carries_its_code_and_message() {
        let err = Error::from(std::io::Error::other("boom"));

        assert_eq!(err.to_string(), "io error: boom");
        assert_eq!(
            err.code().map(|c| c.to_string()).as_deref(),
            Some("arbitrary_almanac::io")
        );
    }

    #[test]
    fn lua_error_carries_its_code_and_message() {
        let err = Error::from(mlua::Error::runtime("boom"));

        assert_eq!(err.to_string(), "lua error: runtime error: boom");
        assert_eq!(
            err.code().map(|c| c.to_string()).as_deref(),
            Some("arbitrary_almanac::lua")
        );
    }
}
