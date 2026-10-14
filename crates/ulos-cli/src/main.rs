use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use ulos_core::tasks::{HttpDownloadTask, TransferState};

#[derive(Parser)]
#[command(
    name = "ulos",
    version,
    about = "Download an HTTP(S) file without overwriting existing files"
)]
struct Args {
    /// Source URL. Prefer public URLs: shell arguments may expose signed links.
    url: String,
    /// Explicit destination filename. Its parent directory must already exist.
    #[arg(short, long)]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut transfer =
        HttpDownloadTask::start(&tokio::runtime::Handle::current(), args.url, args.output);
    let mut updates = transfer.subscribe();
    let mut cancelling = false;
    loop {
        let state = updates.borrow_and_update().clone();
        match state {
            TransferState::Completed(receipt) => {
                println!("Saved {} bytes to {}", receipt.downloaded_bytes, receipt.destination.display());
                return Ok(());
            }
            TransferState::Failed(error) => return Err(error.into()),
            TransferState::Cancelled => return Err("Download cancelled. A completed file may exist if finalization had already started".into()),
            _ => {}
        }
        tokio::select! {
            result = updates.changed() => {
                result.map_err(|_| "Transfer worker stopped before reporting an outcome")?;
            }
            result = tokio::signal::ctrl_c(), if !cancelling => {
                result?;
                transfer.cancel();
                cancelling = true;
            }
        }
    }
}
