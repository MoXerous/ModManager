use std::time::Duration;

use clap::{Parser, Subcommand};
use mm_core::demo::{self, Progress};

#[derive(Parser)]
#[command(name = "modmanager", version, about = "ModManager command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// يعرض الإصدار والنظام
    Info,
    /// يحاكي تحميلاً لإثبات مسار التقدم
    DemoDownload {
        /// الحجم الكلي بالبايت
        #[arg(long, default_value_t = 1_000_000)]
        total: u64,
    },
}

fn main() {
    match Cli::parse().command {
        Command::Info => {
            let info = mm_core::app_info();
            println!(
                "{} {} ({} / {})",
                info.name, info.version, info.os, info.arch
            );
        }
        Command::DemoDownload { total } => {
            demo::run(
                total,
                10,
                Duration::from_millis(50),
                |progress| match progress {
                    Progress::Started { total } => println!("started: {total} bytes"),
                    Progress::Advanced { downloaded } => println!("downloaded: {downloaded}"),
                    Progress::Finished => println!("finished"),
                },
            );
        }
    }
}
