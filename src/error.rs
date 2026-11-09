use miette::Diagnostic;

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("io error: {0}")]
    #[diagnostic(code(arbitrary_almanac::io))]
    Io(#[from] std::io::Error),
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
}
