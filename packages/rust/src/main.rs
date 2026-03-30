mod ytmusic;

use chrono::Local;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use ytmusic::YTMusic;

#[derive(Parser)]
#[command(name = "spotify-to-ytmusic")]
#[command(about = "Import Spotify CSV exports into YouTube Music")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Set up YouTube Music browser authentication
    Setup {
        /// Path to a JSON file with YouTube Music browser headers
        #[arg(long)]
        headers_json: Option<PathBuf>,

        /// Path to save auth file
        #[arg(long, default_value = "browser.json")]
        auth: PathBuf,
    },
    /// Import Spotify CSV exports into YouTube Music
    Import {
        /// Path to folder with Spotify CSV files
        path: PathBuf,

        /// Path to YouTube Music auth JSON
        #[arg(long, default_value = "browser.json")]
        auth: PathBuf,
    },
    /// Retry failed imports from a previous failure log
    Retry {
        /// Path to a failed_*.json log file
        failure_log: PathBuf,

        /// Path to YouTube Music auth JSON
        #[arg(long, default_value = "browser.json")]
        auth: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Track {
    name: String,
    artist: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailureLog {
    #[serde(default)]
    liked_songs: Vec<Track>,
    #[serde(default)]
    playlists: HashMap<String, Vec<Track>>,
}

impl FailureLog {
    fn new() -> Self {
        Self {
            liked_songs: Vec::new(),
            playlists: HashMap::new(),
        }
    }

    fn has_failures(&self) -> bool {
        !self.liked_songs.is_empty() || !self.playlists.is_empty()
    }
}

fn load_csv(path: &std::path::Path) -> Vec<Track> {
    let mut tracks = Vec::new();
    let Ok(mut reader) = csv::Reader::from_path(path) else {
        return tracks;
    };
    for result in reader.records() {
        let Ok(record) = result else { continue };
        let name = record.get(1).unwrap_or("").trim().to_string();
        let artist = record
            .get(3)
            .unwrap_or("")
            .trim()
            .split(';')
            .next()
            .unwrap_or("")
            .to_string();
        if !name.is_empty() && !artist.is_empty() {
            tracks.push(Track { name, artist });
        }
    }
    tracks
}

fn do_setup(auth_path: &std::path::Path, headers_json: Option<&std::path::Path>) {
    if headers_json.is_none() && auth_path.exists() {
        println!("Existing auth found at {}", auth_path.display());
        println!("To re-authenticate, delete the file and run setup again.");
        return;
    }

    if let Some(json_path) = headers_json {
        if !json_path.exists() {
            eprintln!("File not found: {}", json_path.display());
            std::process::exit(1);
        }
        let content = std::fs::read_to_string(json_path).expect("Failed to read headers JSON");
        let headers: serde_json::Value =
            serde_json::from_str(&content).expect("Invalid JSON in headers file");
        let out = serde_json::to_string_pretty(&headers).unwrap();
        std::fs::write(auth_path, out).expect("Failed to write auth file");
        println!(
            "Auth imported from {} -> {}",
            json_path.display(),
            auth_path.display()
        );
        return;
    }

    println!("YouTube Music Authentication Setup");
    println!("{}", "=".repeat(40));
    println!();
    println!("See: https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html");
    println!();
    println!("Paste your browser headers as JSON (e.g. from browser DevTools),");
    println!("then press Enter followed by Ctrl+Z then Enter (Windows) or Ctrl+D (Mac/Linux):");
    println!();

    let mut input = String::new();
    loop {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => input.push_str(&line),
            Err(_) => break,
        }
    }

    let headers: serde_json::Value = match serde_json::from_str(input.trim()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to parse headers JSON: {e}");
            eprintln!("Make sure you paste valid JSON with cookie and authorization fields.");
            std::process::exit(1);
        }
    };

    let obj = headers.as_object().expect("Headers must be a JSON object");
    for key in ["cookie", "authorization"] {
        if !obj.contains_key(key) {
            eprintln!("Missing required header: {key}");
            std::process::exit(1);
        }
    }

    let out = serde_json::to_string_pretty(&headers).unwrap();
    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(auth_path, out).expect("Failed to write auth file");
    println!("\nAuth saved to: {}", auth_path.display());
    println!("You can now run: spotify-to-ytmusic import /path/to/csv/folder");
}

async fn import_liked_songs(yt: &YTMusic, tracks: &[Track]) -> (usize, Vec<Track>) {
    println!("\n=== Importing {} liked songs ===\n", tracks.len());
    let mut success = 0;
    let mut failed = Vec::new();

    for (i, track) in tracks.iter().enumerate() {
        let idx = i + 1;
        match yt.search_song(&track.name, &track.artist).await {
            Some(video_id) => match yt.rate_song(&video_id, "LIKE").await {
                Ok(()) => {
                    println!(
                        "  [{}/{}] Liked: {} - {}",
                        idx,
                        tracks.len(),
                        track.artist,
                        track.name
                    );
                    success += 1;
                }
                Err(e) => {
                    println!(
                        "  [{}/{}] Error liking '{}': {}",
                        idx,
                        tracks.len(),
                        track.name,
                        e
                    );
                    failed.push(track.clone());
                }
            },
            None => {
                println!(
                    "  [{}/{}] Not found: {} - {}",
                    idx,
                    tracks.len(),
                    track.artist,
                    track.name
                );
                failed.push(track.clone());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    (success, failed)
}

async fn import_playlist(
    yt: &YTMusic,
    playlist_name: &str,
    tracks: &[Track],
) -> (usize, usize, Vec<Track>) {
    println!(
        "\n=== Importing playlist '{}' ({} songs) ===\n",
        playlist_name,
        tracks.len()
    );
    let mut video_ids = Vec::new();
    let mut failed = Vec::new();

    for (i, track) in tracks.iter().enumerate() {
        let idx = i + 1;
        match yt.search_song(&track.name, &track.artist).await {
            Some(video_id) => {
                println!(
                    "  [{}/{}] Found: {} - {}",
                    idx,
                    tracks.len(),
                    track.artist,
                    track.name
                );
                video_ids.push(video_id);
            }
            None => {
                println!(
                    "  [{}/{}] Not found: {} - {}",
                    idx,
                    tracks.len(),
                    track.artist,
                    track.name
                );
                failed.push(track.clone());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    if !video_ids.is_empty() {
        match yt.find_playlist(playlist_name).await {
            Some(existing_id) => {
                let existing_videos = yt.get_playlist_video_ids(&existing_id).await;
                let new_ids: Vec<String> = video_ids
                    .iter()
                    .filter(|id| !existing_videos.contains(id.as_str()))
                    .cloned()
                    .collect();
                let skipped = video_ids.len() - new_ids.len();
                let added = new_ids.len();
                if !new_ids.is_empty() {
                    match yt.add_playlist_items(&existing_id, &new_ids).await {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!("\n  Error updating playlist: {}", e);
                            return (0, 0, failed);
                        }
                    }
                }
                return (added, skipped, failed);
            }
            None => {
                let description = format!("Imported from Spotify ({} songs)", video_ids.len());
                match yt
                    .create_playlist(playlist_name, &description, &video_ids)
                    .await
                {
                    Ok(_) => {
                        return (video_ids.len(), 0, failed);
                    }
                    Err(e) => {
                        eprintln!("\n  Error creating playlist: {}", e);
                        return (0, 0, failed);
                    }
                }
            }
        }
    }

    (0, 0, failed)
}

fn write_failure_log(failures: &FailureLog) -> PathBuf {
    let output_dir = PathBuf::from("output");
    std::fs::create_dir_all(&output_dir).expect("Failed to create output directory");

    let timestamp = Local::now().format("%Y-%m-%d_%H%M%S");
    let log_path = output_dir.join(format!("failed_{}.json", timestamp));

    let json = serde_json::to_string_pretty(failures).unwrap();
    std::fs::write(&log_path, json).expect("Failed to write failure log");
    log_path
}

fn print_failures(failures: &FailureLog) {
    if !failures.liked_songs.is_empty() {
        println!("\n  Liked Songs:");
        for t in &failures.liked_songs {
            println!("    - {} - {}", t.artist, t.name);
        }
    }
    for (playlist, tracks) in &failures.playlists {
        println!("\n  {}:", playlist);
        for t in tracks {
            println!("    - {} - {}", t.artist, t.name);
        }
    }
}

fn prompt_choice() -> String {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .expect("Failed to read input");
    input.trim().to_string()
}

async fn do_import(csv_dir: &std::path::Path, auth_path: &std::path::Path) {
    if !auth_path.exists() {
        eprintln!("Auth file not found: {}", auth_path.display());
        eprintln!("Run 'spotify-to-ytmusic setup' first.");
        std::process::exit(1);
    }

    if !csv_dir.is_dir() {
        eprintln!("Not a directory: {}", csv_dir.display());
        std::process::exit(1);
    }

    let mut csv_files: Vec<_> = std::fs::read_dir(csv_dir)
        .expect("Failed to read directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "csv"))
        .collect();
    csv_files.sort();

    if csv_files.is_empty() {
        eprintln!("No CSV files found in {}", csv_dir.display());
        std::process::exit(1);
    }

    println!("Connecting to YouTube Music...");
    let yt = YTMusic::new(auth_path);

    let liked_file = csv_dir.join("Liked_Songs.csv");
    let has_liked = liked_file.exists();
    let playlist_files: Vec<_> = csv_files
        .iter()
        .filter(|p| p.file_name().unwrap().to_str() != Some("Liked_Songs.csv"))
        .collect();

    println!("\nFound {} CSV files:", csv_files.len());
    if has_liked {
        let tracks = load_csv(&liked_file);
        println!("  - Liked Songs ({} songs)", tracks.len());
    }
    for f in &playlist_files {
        let tracks = load_csv(f);
        let name = f
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .replace('_', " ");
        println!("  - {} ({} songs)", name, tracks.len());
    }

    println!("\nOptions:");
    println!("  1) Import everything (liked songs + all playlists)");
    if has_liked {
        println!("  2) Import liked songs only");
    }
    println!("  3) Import playlists only");
    print!("\nChoice [1/2/3]: ");
    use std::io::Write;
    std::io::stdout().flush().ok();

    let choice = prompt_choice();
    let mut failures = FailureLog::new();

    if (choice == "1" || choice == "2") && has_liked {
        let tracks = load_csv(&liked_file);
        let (success, failed) = import_liked_songs(&yt, &tracks).await;
        println!("\n  Liked songs: {} added, {} failed", success, failed.len());
        failures.liked_songs = failed;
    }

    if choice == "1" || choice == "3" {
        for f in &playlist_files {
            let tracks = load_csv(f);
            if tracks.is_empty() {
                continue;
            }
            let name = f
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .replace('_', " ");
            let (added, skipped, failed) = import_playlist(&yt, &name, &tracks).await;
            let mut parts = Vec::new();
            if added > 0 { parts.push(format!("{} added", added)); }
            if skipped > 0 { parts.push(format!("{} skipped", skipped)); }
            if !failed.is_empty() { parts.push(format!("{} failed", failed.len())); }
            println!("\n  '{}': {}", name, if parts.is_empty() { "nothing to do".into() } else { parts.join(", ") });
            if !failed.is_empty() {
                failures.playlists.insert(name, failed);
            }
        }
    }

    if failures.has_failures() {
        let log_path = write_failure_log(&failures);
        println!(
            "\n\n=== Songs not found (logged to {}) ===",
            log_path.display()
        );
        print_failures(&failures);
        println!(
            "\nRetry with: spotify-to-ytmusic retry {}",
            log_path.display()
        );
    }

    println!("\nDone!");
}

async fn do_retry(failure_log: &std::path::Path, auth_path: &std::path::Path) {
    if !auth_path.exists() {
        eprintln!("Auth file not found: {}", auth_path.display());
        eprintln!("Run 'spotify-to-ytmusic setup' first.");
        std::process::exit(1);
    }

    if !failure_log.exists() {
        eprintln!("File not found: {}", failure_log.display());
        std::process::exit(1);
    }

    let content =
        std::fs::read_to_string(failure_log).expect("Failed to read failure log");
    let data: FailureLog =
        serde_json::from_str(&content).expect("Invalid failure log format");

    let total: usize =
        data.liked_songs.len() + data.playlists.values().map(|t| t.len()).sum::<usize>();
    if total == 0 {
        println!("No failed tracks to retry.");
        return;
    }

    println!(
        "Retrying {} failed tracks from {}...",
        total,
        failure_log
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
    );
    if !data.liked_songs.is_empty() {
        println!("  - Liked Songs ({} songs)", data.liked_songs.len());
    }
    for (name, tracks) in &data.playlists {
        println!("  - {} ({} songs)", name, tracks.len());
    }

    println!("\nConnecting to YouTube Music...");
    let yt = YTMusic::new(auth_path);

    let mut failures = FailureLog::new();

    if !data.liked_songs.is_empty() {
        let (success, failed) = import_liked_songs(&yt, &data.liked_songs).await;
        println!(
            "\n  Liked songs: {} added, {} still failing",
            success,
            failed.len()
        );
        failures.liked_songs = failed;
    }

    for (name, tracks) in &data.playlists {
        let (added, skipped, failed) = import_playlist(&yt, name, tracks).await;
        let mut parts = Vec::new();
        if added > 0 { parts.push(format!("{} added", added)); }
        if skipped > 0 { parts.push(format!("{} skipped", skipped)); }
        if !failed.is_empty() { parts.push(format!("{} failed", failed.len())); }
        println!("\n  '{}': {}", name, if parts.is_empty() { "nothing to do".into() } else { parts.join(", ") });
        if !failed.is_empty() {
            failures.playlists.insert(name.clone(), failed);
        }
    }

    if failures.has_failures() {
        let log_path = write_failure_log(&failures);
        println!(
            "\n\n=== Still failing (logged to {}) ===",
            log_path.display()
        );
        print_failures(&failures);
        println!(
            "\nRetry again with: spotify-to-ytmusic retry {}",
            log_path.display()
        );
    } else {
        println!("\nAll tracks imported successfully!");
    }

    println!("\nDone!");
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Setup {
            headers_json,
            auth,
        } => {
            do_setup(&auth, headers_json.as_deref());
        }
        Commands::Import { path, auth } => {
            do_import(&path, &auth).await;
        }
        Commands::Retry { failure_log, auth } => {
            do_retry(&failure_log, &auth).await;
        }
    }
}
