use miette::{Diagnostic, NamedSource, SourceSpan};

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
        src: NamedSource<String>,
        #[label("here")]
        at: Option<SourceSpan>,
    },

    #[error("a workflow must end with {{: declare : gather : policy}}")]
    #[diagnostic(code(arbitrary_almanac::exports))]
    Exports {
        #[source_code]
        src: NamedSource<String>,
        #[label("the last form")]
        at: Option<SourceSpan>,
    },

    #[error("a workflow must declare itself once, with (local declare {{...}})")]
    #[diagnostic(code(arbitrary_almanac::declare))]
    Undeclared {
        #[source_code]
        src: NamedSource<String>,
    },

    #[error("a declaration is read without running it, so it must be a literal")]
    #[diagnostic(code(arbitrary_almanac::literal))]
    NotLiteral {
        #[source_code]
        src: NamedSource<String>,
        #[label("not a literal")]
        at: Option<SourceSpan>,
    },

    #[error("invalid declaration: {message}")]
    #[diagnostic(code(arbitrary_almanac::declaration))]
    Declaration {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("this declaration")]
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
