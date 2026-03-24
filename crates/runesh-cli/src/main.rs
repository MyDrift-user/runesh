use clap::{Parser, Subcommand};

mod init;

/// Default GitHub repo for RUNESH shared code.
/// Override with --repo flag or RUNESH_REPO env var.
pub const DEFAULT_REPO: &str = "https://github.com/mydrift-user/runesh";
pub const DEFAULT_NPM_SCOPE: &str = "@mydrift-user";

#[derive(Parser)]
#[command(name = "runesh", version, about = "Scaffold and manage RUNESH-based projects")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new project in the current directory (or a new subdirectory)
    Init {
        /// Initialize in a new subdirectory with this name.
        /// If omitted, initializes in the current directory.
        name: Option<String>,

        /// GitHub repo URL for RUNESH (for Cargo git deps).
        #[arg(long)]
        repo: Option<String>,

        /// Use local file paths instead of git/npm references.
        #[arg(long)]
        local: bool,

        /// Local path to the RUNESH repo (only used with --local).
        #[arg(long)]
        runesh_path: Option<String>,

        /// Skip interactive prompts, use defaults (server + web + auth + rate limit + openapi + docker).
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, repo, local, runesh_path, yes } => {
            if let Err(e) = init::run(name, repo, local, runesh_path, yes) {
                eprintln!("\x1b[31merror:\x1b[0m {e}");
                std::process::exit(1);
            }
        }
    }
}
