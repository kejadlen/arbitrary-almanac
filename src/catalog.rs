//! What this binary ships for workflows to use. Today that is the providers `gather` may be
//! handed. Events and prompts join it as the host grows.

/// Everything a workflow may name. A name not here, and not one of the workflow's own agents,
/// refuses the workflow at load.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub providers: Vec<&'static str>,
}
