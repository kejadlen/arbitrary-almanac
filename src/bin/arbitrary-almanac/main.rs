use almanac::workflow::Registered;
use arbitrary_almanac::Catalog;
use arbitrary_almanac::bundle::{Bundle, build};
use arbitrary_almanac::host::Registration;
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
    /// Check the workflows in DIR and write a binary with them bundled to OUTPUT.
    Build {
        dir: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// The generic binary to bundle into, such as one built for another platform. Defaults
        /// to this one.
        #[arg(long)]
        binary: Option<PathBuf>,
    },
    /// One of Almanac's verbs, such as `synth`, `check`, `run`, or `simulate KIND FILE`, run on
    /// the bundled workflows, or else those in `$ARBITRARY_ALMANAC_WORKFLOWS` (default
    /// `workflows`).
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

    let catalog = Catalog::shipped();
    let this = std::env::current_exe().into_diagnostic()?;
    match cli.verb {
        None => {
            Cli::command().print_help().into_diagnostic()?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Verb::Build {
            dir,
            output,
            binary,
        }) => {
            let binary = binary.unwrap_or_else(|| this.clone());
            let bundle = build(&catalog, &dir, &binary, &output)?;
            let count = bundle.workflows.len();
            println!("bundled {count} workflow(s) into {}", output.display());
            return Ok(ExitCode::SUCCESS);
        }
        Some(Verb::Almanac(_)) => {}
    }
    // A bundled binary runs its bundle. Without one, it reads a directory.
    let bundle = match Bundle::of_binary(&this)? {
        Some(bundle) => bundle,
        None => {
            let dir = std::env::var_os("ARBITRARY_ALMANAC_WORKFLOWS")
                .map_or_else(|| PathBuf::from("workflows"), PathBuf::from);
            Bundle::read_dir(&dir)?
        }
    };
    let loaded = bundle.load(&catalog)?;
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
