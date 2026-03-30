use clap::{Parser, Subcommand};

mod init;
mod new_project;

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

    /// Create a new project repo with RUNESH integration, git init, and optional GitHub repo
    New {
        /// Project name (will be the directory and repo name).
        name: String,

        /// Short description of the project.
        #[arg(long, short = 'd')]
        description: Option<String>,

        /// RUNESH crates to include (comma-separated, e.g. "core,auth,inventory").
        /// Available: core, auth, inventory, remote, desktop, vfs, tun, mesh
        #[arg(long, short = 'c')]
        crates: Option<String>,

        /// Create a GitHub repo (requires `gh` CLI to be installed and authenticated).
        #[arg(long)]
        github: bool,

        /// Make the GitHub repo private (default: private).
        #[arg(long, default_value_t = true)]
        private: bool,

        /// GitHub org to create the repo under (if omitted, uses your personal account).
        #[arg(long)]
        org: Option<String>,

        /// Use local RUNESH path references instead of git.
        #[arg(long)]
        local: bool,

        /// Skip interactive prompts, use defaults.
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
        Commands::New { name, description, crates, github, private, org, local, yes } => {
            if let Err(e) = new_project::run(name, description, crates, github, private, org, local, yes) {
                eprintln!("\x1b[31merror:\x1b[0m {e}");
                std::process::exit(1);
            }
        }
    }
}
