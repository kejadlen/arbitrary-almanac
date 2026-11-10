use almanac::workflow::Registered;
use arbitrary_almanac::Catalog;
use arbitrary_almanac::host::{Registration, load_dir};
use clap::CommandFactory as _;
use clap::Parser as _;
use clap_complete::Shell;
use miette::IntoDiagnostic as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use tracing_subscriber::prelude::*;

const VERSION: &str = env!("ARBITRARY_ALMANAC_VERSION");

#[derive(clap::Parser)]
#[command(
    name = "arbitrary-almanac",
    about = "A personal almanac built on Almanac",
    version = VERSION
)]
struct Cli {
    /// Generate shell completions and exit.
    #[arg(long, value_enum)]
    completions: Option<Shell>,

    #[command(subcommand)]
    verb: Option<Verb>,
}

#[derive(clap::Subcommand)]
enum Verb {
    /// One of Almanac's verbs, such as `synth`, `check`, `run`, or `simulate KIND FILE`, run on
    /// the workflows in `$ARBITRARY_ALMANAC_WORKFLOWS` (default `workflows`).
    #[command(external_subcommand)]
    Almanac(
        #[expect(
            dead_code,
            reason = "Almanac reads its verb from the process's arguments"
        )]
        Vec<String>,
    ),
}

/// The loaded workflows, handed to Almanac's runtime once.
static LOADED: Mutex<Vec<Box<dyn Registered>>> = Mutex::new(Vec::new());

fn workflows() -> Vec<Box<dyn Registered>> {
    LOADED
        .lock()
        .map(|mut loaded| std::mem::take(&mut *loaded))
        .unwrap_or_default()
}

fn main() -> miette::Result<ExitCode> {
    miette::set_panic_hook();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    if let Some(shell) = cli.completions {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "arbitrary-almanac",
            &mut std::io::stdout(),
        );
        return Ok(ExitCode::SUCCESS);
    }

    let Some(Verb::Almanac(_)) = cli.verb else {
        Cli::command().print_help().into_diagnostic()?;
        return Ok(ExitCode::SUCCESS);
    };
    let dir = std::env::var_os("ARBITRARY_ALMANAC_WORKFLOWS")
        .map_or_else(|| PathBuf::from("workflows"), PathBuf::from);
    let loaded = load_dir(&Catalog::shipped(), &dir)?;
    if let Ok(mut slot) = LOADED.lock() {
        *slot = loaded
            .into_iter()
            .map(|workflow| Box::new(Registration::new(workflow)) as Box<dyn Registered>)
            .collect();
    }
    // Almanac reads the verb from the arguments itself.
    Ok(almanac::main_with(
        "arbitrary-almanac",
        workflows,
        almanac_renovate::providers::all,
        &[],
    ))
}
