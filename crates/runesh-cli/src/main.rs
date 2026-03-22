use clap::{Parser, Subcommand};

mod init;

#[derive(Parser)]
#[command(name = "runesh", version, about = "Scaffold and manage RUNESH-based projects")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new project with RUNESH shared code
    Init {
        /// Project name (e.g. "my-app"). Creates a directory with this name.
        name: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name } => {
            if let Err(e) = init::run(name) {
                eprintln!("\x1b[31merror:\x1b[0m {e}");
                std::process::exit(1);
            }
        }
    }
}
