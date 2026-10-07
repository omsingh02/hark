use std::path::PathBuf;
use clap::{Parser, Subcommand};
use jiosaavn::JioSaavnClient;

#[derive(Parser)]
#[command(name = "jiosaavn")]
#[command(about = "Standalone production-grade JioSaavn music search, streaming & downloader", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search JioSaavn catalog and list ranked candidates
    Search {
        query: String,
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },
    /// Resolve direct CDN stream URL with bitrate waterfall (320 -> 160 -> 128 -> 96)
    StreamUrl {
        title: String,
        artist: String,
    },
    /// Download and tag song with full metadata & album art
    Download {
        title: String,
        artist: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Ensure song is present and dispatch to default player (MPD / mpc)
    Play {
        title: String,
        artist: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let client = JioSaavnClient::new();

    match cli.command {
        Commands::Search { query, limit } => {
            println!("Searching for '{}'...", query);
            let results = client.search(&query, limit).await?;
            if results.is_empty() {
                println!("No results found.");
                return Ok(());
            }

            for (i, song) in results.iter().enumerate() {
                println!(
                    "[{}] {} - {} (Album: {}, Year: {}, 320k: {})",
                    i + 1,
                    song.clean_title(),
                    song.clean_artist(),
                    song.clean_album().as_deref().unwrap_or("N/A"),
                    song.year.as_deref().unwrap_or("N/A"),
                    song.is_320kbps.as_deref().unwrap_or("false")
                );
            }
        }
        Commands::StreamUrl { title, artist } => {
            let song = client.find_best_match(&title, &artist).await?;
            let info = client.get_stream_url(&song).await?;
            println!("Matched Track: {} - {}", song.clean_title(), song.clean_artist());
            println!("Bitrate: {}kbps", info.bitrate_kbps);
            println!("Stream URL: {}", info.url);
        }
        Commands::Download { title, artist, out } => {
            let out_dir = out.unwrap_or_else(JioSaavnClient::get_music_dir);
            println!("Finding & downloading '{} - {}' to {}...", title, artist, out_dir.display());
            let song = client.find_best_match(&title, &artist).await?;
            let path = client.download_song(&song, &out_dir).await?;
            println!("Successfully saved: {}", path.display());
        }
        Commands::Play { title, artist, out } => {
            let out_dir = out.unwrap_or_else(JioSaavnClient::get_music_dir);
            let path = client.ensure_song(&title, &artist, &out_dir).await?;
            let msg = client.play_in_default_player(&path).await?;
            println!("{}", msg);
        }
    }

    Ok(())
}
