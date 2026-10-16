use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod commands;

/// ModManager command line: thin client of mm-core. Everything mutating
/// is plan-first; `plan` (and `--dry-run`) never writes.
#[derive(Parser)]
#[command(name = "modmanager", version, about = "ModManager command line")]
struct Cli {
    /// Instance directory (its `mods/` child holds the jars).
    #[arg(long, global = true)]
    instance: Option<PathBuf>,
    /// Modrinth API base (hidden, tests only: points at wiremock).
    #[arg(long, global = true, hide = true)]
    api_base: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show version and platform.
    Info,
    /// Search Modrinth projects (read-only).
    Search {
        query: String,
        #[arg(long)]
        game: Option<String>,
        #[arg(long)]
        loader: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// List scanned mod files (read-only).
    Scan,
    /// List locked (installed) mods (read-only).
    List,
    /// Resolve slugs and print the install preview (never writes).
    Plan {
        #[arg(required = true)]
        slugs: Vec<String>,
        #[arg(long)]
        game: Option<String>,
        #[arg(long)]
        loader: Option<String>,
    },
    /// Install mods: resolve → confirm → download → write lock.
    Install {
        #[arg(required = true)]
        slugs: Vec<String>,
        #[arg(long)]
        game: Option<String>,
        #[arg(long)]
        loader: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Update locked mods to the newest compatible versions.
    Update {
        #[arg(long)]
        game: Option<String>,
        #[arg(long)]
        loader: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove mods: move jars to a backup dir, drop lock entries.
    Remove {
        #[arg(required = true)]
        selectors: Vec<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = match run(cli).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            e.code()
        }
    };
    std::process::exit(code);
}

async fn run(cli: Cli) -> Result<(), commands::CliError> {
    let ctx = commands::Ctx {
        instance: cli.instance,
        api_base: cli.api_base,
    };
    match cli.command {
        Command::Info => commands::info(),
        Command::Search {
            query,
            game,
            loader,
            limit,
        } => commands::search(&ctx, &query, game, loader, limit).await,
        Command::Scan => commands::scan(&ctx).await,
        Command::List => commands::list(&ctx).await,
        Command::Plan {
            slugs,
            game,
            loader,
        } => commands::plan(&ctx, &slugs, game, loader).await,
        Command::Install {
            slugs,
            game,
            loader,
            yes,
            dry_run,
        } => commands::install(&ctx, &slugs, game, loader, yes, dry_run).await,
        Command::Update {
            game,
            loader,
            yes,
            dry_run,
        } => commands::update(&ctx, game, loader, yes, dry_run).await,
        Command::Remove {
            selectors,
            yes,
            dry_run,
        } => commands::remove(&ctx, &selectors, yes, dry_run).await,
    }
}
