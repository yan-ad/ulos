use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use ulos_core::{DownloadRequest, Downloader};

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
    let request = DownloadRequest::new(&args.url, args.output)?;
    let downloader = Downloader::new()?;
    let transfer = downloader.download(&request, |_| {});
    tokio::pin!(transfer);
    tokio::select! {
        result = &mut transfer => {
            let receipt = result?;
            println!("Saved {} bytes to {}", receipt.downloaded_bytes, receipt.destination.display());
            Ok(())
        }
        result = tokio::signal::ctrl_c() => {
            result?;
            Err("Download cancelled. A completed file may exist if finalization had already started".into())
        }
    }
}
