use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "remake-dev", version)]
#[command(about = "RemakeBackend development tools: local pgrust, backend, tests and reports")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Machine-readable output: one JSON document on stdout (cargo's own output goes to stderr).
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create `.devtools/`, a default `devtools.toml` and the .gitignore entry
    Setup,
    /// Start the backend, reusing the local pgrust, and restart it when its code changes
    Dev {
        /// Also run the admin UI (Vite) on 127.0.0.1:5173
        #[arg(long)]
        ui: bool,
        /// Do not watch for changes
        #[arg(long)]
        no_watch: bool,
        /// Use the database at DATABASE_URL as is (e.g. Docker Compose); do not start pgrust
        #[arg(long)]
        no_db: bool,
        /// Do not apply migrations before starting
        #[arg(long)]
        no_migrate: bool,
    },
    /// Build the backend only (`cargo build -p cloud-api`)
    Build {
        #[arg(long)]
        release: bool,
    },
    /// Fast checks over the backend packages
    Check {
        /// Run clippy instead of cargo check
        #[arg(long)]
        clippy: bool,
    },
    /// Run tests: unit by default, no database involved
    Test {
        #[arg(long)]
        unit: bool,
        /// Database tests against the isolated test database
        #[arg(long)]
        integration: bool,
        /// Every configured category
        #[arg(long)]
        all: bool,
    },
    /// Check the environment: toolchain, pgrust install, instance, migrations, build-cache factors
    Doctor,
    /// Write and print a project report
    Report {
        /// json or markdown
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Manage the local pgrust
    Db {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
    /// Remove generated logs, reports and recorded results (never the install or the data)
    Clean,
}

#[derive(Subcommand)]
pub enum DatabaseCommand {
    /// Install pgrust. The only place besides `upgrade` where pgrust is ever compiled
    Install {
        /// A pgrust checkout to build from (default: vendor/pgrust)
        #[arg(long, conflicts_with = "prebuilt")]
        source: Option<PathBuf>,
        /// An already built release directory (postgres, initdb, share/) to copy instead of building
        #[arg(long)]
        prebuilt: Option<PathBuf>,
        /// Replace an existing installation
        #[arg(long)]
        force: bool,
        /// Build without main_main/full (faster; no CJK, pgcrypto or Snappy/Brotli)
        #[arg(long)]
        slim: bool,
    },
    /// Start the installed pgrust, or reuse the one already running. Never compiles
    Up,
    /// Stop the managed instance (data is kept)
    Down,
    Restart,
    /// Connection, pid, version and health
    Status,
    /// Tail the server log
    Logs {
        #[arg(long, default_value_t = 50)]
        lines: usize,
    },
    /// Recreate the isolated test database (never the development one)
    Reset {
        #[arg(long)]
        yes: bool,
    },
    /// Replace the installed pgrust with a newer build
    Upgrade {
        #[arg(long, conflicts_with = "prebuilt")]
        source: Option<PathBuf>,
        #[arg(long)]
        prebuilt: Option<PathBuf>,
        #[arg(long)]
        slim: bool,
        /// Allow a different server version while a data directory exists
        #[arg(long)]
        accept_data_risk: bool,
    },
}
