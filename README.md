<p align="center">
  <img src="icon.svg" alt="spotify-to-ytmusic" width="128">
</p>

# Spotify to YouTube Music Importer

Transfer your Spotify playlists and liked songs to YouTube Music using CSV exports. Available in both Python and Rust.

## Project Structure

```
packages/
  python/   # Python variant (requires ytmusicapi)
  rust/     # Rust variant (standalone, no Python needed)
```

## Prerequisites

- A YouTube Music account
- **Python variant:** Python 3.9+, `pip install ytmusicapi`
- **Rust variant:** Rust toolchain (`cargo`)

## Setup

### 1. Export your Spotify playlists

Go to [Exportify](https://exportify.net), log in with your Spotify account, and export your playlists as CSV files into `data/`.

The tool expects CSVs with these columns (Exportify's default format):

| Column | Description |
|--------|-------------|
| Track Name | Song title |
| Artist Name(s) | Artist(s), semicolon-separated |

A file named `Liked_Songs.csv` is treated specially — its tracks are added to your YouTube Music liked songs instead of a playlist.

### 2. Authenticate with YouTube Music

You need browser credentials to access the YouTube Music API. See the [ytmusicapi browser auth guide](https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html) for full details.

**Option A: Interactive setup (paste raw headers)**

```bash
# Python
python packages/python/import_to_ytmusic.py setup

# Rust
cargo run --manifest-path packages/rust/Cargo.toml -- setup
```

1. Open [YouTube Music](https://music.youtube.com) in your browser (make sure you're logged in)
2. Open DevTools (`F12`) → **Network** tab
3. Click on any request to `music.youtube.com`
4. Right-click the request → **Copy** → **Copy request headers**
5. Paste into the terminal, then press `Enter` followed by `Ctrl+Z, Enter` (Windows) or `Ctrl+D` (Mac/Linux)

**Option B: Import from a JSON file**

If you already have a headers JSON file (e.g. from ytmusicapi):

```bash
# Python
python packages/python/import_to_ytmusic.py setup --headers-json /path/to/browser.json

# Rust
cargo run --manifest-path packages/rust/Cargo.toml -- setup --headers-json /path/to/browser.json
```

Auth credentials are saved to `browser.json` in the project root (gitignored).

## Usage

### Import

```bash
# Python
python packages/python/import_to_ytmusic.py import data/

# Rust
cargo run --manifest-path packages/rust/Cargo.toml -- import data/
```

The tool will show you what it found and let you choose:

```
Found 8 CSV files:
  - Liked Songs (429 songs)
  - Pandemonium (114 songs)
  - Empathy (47 songs)
  ...

Options:
  1) Import everything (liked songs + all playlists)
  2) Import liked songs only
  3) Import playlists only
```

### Retry failed imports

If some songs fail (not found on YouTube Music or rate limit errors), a failure log is saved to `output/failed_<timestamp>.json`. Retry with:

```bash
# Python
python packages/python/import_to_ytmusic.py retry output/failed_2026-03-30_201500.json

# Rust
cargo run --manifest-path packages/rust/Cargo.toml -- retry output/failed_2026-03-30_201500.json
```

The failure log format is shared between both variants, so you can import with Python and retry with Rust (or vice versa).

### What it does

- **Liked Songs** → added to your YouTube Music liked songs
- **All other CSVs** → creates or updates a YouTube Music playlist with the CSV filename as the playlist name
- **Duplicate handling** → existing playlist tracks are skipped, never added twice
- **Failure logging** → failed tracks are saved to `output/` for retry

## Turborepo

Both packages are managed with [Turborepo](https://turbo.build/):

```bash
pnpm install        # install turbo
pnpm turbo build    # build both packages
pnpm turbo lint     # lint both packages
pnpm turbo test     # test both packages
```

## Behavior

### Playlist matching

The tool matches playlists **by name** between Spotify CSV filenames and YouTube Music. The CSV filename (minus `.csv`, with underscores replaced by spaces) becomes the playlist name.

### Duplicate playlists

- **Pre-existing YouTube Music playlist with the same name as a Spotify playlist:** The tool will **merge into** the existing playlist, adding only songs that aren't already there. It will never delete tracks or create a duplicate playlist. If you want to keep them separate, rename the playlist on YouTube Music (or rename the CSV file) before importing, unless you want them "merged".
- **Multiple YouTube Music playlists with the same name:** The tool matches the **first** playlist returned by the YouTube Music API. There is no guarantee which one that is. To avoid ambiguity, rename conflicting playlists on YouTube Music (or rename the CSV file) before importing.

### Liked songs

- Liking an already-liked song is a no-op on YouTube Music's side — safe to run multiple times.
- If Spotify has duplicate liked songs (e.g. same track from different albums), they may resolve to the same YouTube Music video and appear as fewer total likes.

### Re-running the tool

It is safe to run `import` or `retry` multiple times. Existing playlist tracks are checked before adding, and duplicates are skipped.

## Notes

- Imports are rate-limited (~0.5s per song) to avoid hitting YouTube's API limits
- Song matching is based on searching `"{track name} {artist}"` on YouTube Music — occasionally the wrong version may be matched
- Browser auth tokens expire after some time. If you get auth errors, re-run `setup`

## License

MIT
