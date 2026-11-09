use clap::CommandFactory as _;
use clap::Parser as _;
use clap_complete::Shell;
use miette::IntoDiagnostic as _;
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
}

#[tokio::main]
async fn main() -> miette::Result<()> {
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
        return Ok(());
    }

    Cli::command().print_help().into_diagnostic()?;
    Ok(())
}
