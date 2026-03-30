#!/usr/bin/env python3
"""
spotify-to-ytmusic: Import Spotify CSV exports into YouTube Music.

Usage:
  1. Export playlists from Spotify using https://exportify.net (CSV files)
  2. Run: python import_to_ytmusic.py setup
  3. Run: python import_to_ytmusic.py import /path/to/csv/folder
  4. If some fail: python import_to_ytmusic.py retry output/failed_2026-03-30_201500.json

Requirements:
  pip install ytmusicapi
"""

import argparse
import csv
import json
import sys
import time
from datetime import datetime
from pathlib import Path

from ytmusicapi import YTMusic, setup


PROJECT_DIR = Path(__file__).parent.parent.parent
DEFAULT_AUTH_PATH = PROJECT_DIR / "browser.json"
OUTPUT_DIR = PROJECT_DIR / "output"


def do_setup(auth_path, headers_json=None):
    """Set up YouTube Music browser authentication.

    Can be run interactively (paste raw headers) or with a pre-made JSON file.
    See https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html
    """
    auth_path.parent.mkdir(parents=True, exist_ok=True)

    if not headers_json and auth_path.exists():
        print(f"Existing auth found at {auth_path}")
        print("To re-authenticate, delete the file and run setup again.")
        return

    if headers_json:
        headers_json = Path(headers_json)
        if not headers_json.exists():
            print(f"File not found: {headers_json}")
            sys.exit(1)
        with open(headers_json) as f:
            headers = json.load(f)
        with open(auth_path, "w") as f:
            json.dump(headers, f, ensure_ascii=True, indent=4, sort_keys=True)
        print(f"Auth imported from {headers_json} -> {auth_path}")
        return

    print("YouTube Music Authentication Setup")
    print("=" * 40)
    print()
    print("See: https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html")
    print()
    print("1. Open https://music.youtube.com in your browser (logged in)")
    print("2. Open DevTools (F12) -> Network tab")
    print("3. Click on any request to music.youtube.com")
    print("4. Right-click the request -> Copy -> Copy request headers")
    print(
        "5. Paste the headers below, then press Enter followed by Ctrl+Z then Enter (Windows)"
    )
    print("   or Ctrl+D (Mac/Linux)")
    print()
    print("Alternatively, run with --headers-json to provide a JSON file directly.")
    print()

    try:
        lines = []
        while True:
            try:
                lines.append(input())
            except EOFError:
                break

        raw_headers = "\n".join(lines)
        setup(filepath=str(auth_path), headers_raw=raw_headers)
        print(f"\nAuth saved to: {auth_path}")
        print("You can now run: python import_to_ytmusic.py import /path/to/csv/folder")
    except Exception as e:
        print(f"\nSetup failed: {e}")
        print("Make sure you copied the full request headers and are logged in.")
        sys.exit(1)


def load_csv(filepath):
    """Load tracks from a Spotify/Exportify CSV export."""
    tracks = []
    with open(filepath, encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            name = row.get("Track Name", "").strip()
            artist = row.get("Artist Name(s)", "").strip().split(";")[0]
            if name and artist:
                tracks.append({"name": name, "artist": artist})
    return tracks


def search_song(yt, name, artist):
    """Search YouTube Music for a song, return video ID or None."""
    query = f"{name} {artist}"
    try:
        results = yt.search(query, filter="songs", limit=3)
        if results:
            return results[0]["videoId"]
    except Exception as e:
        print(f"  Search error for '{query}': {e}")
    return None


def import_liked_songs(yt, tracks):
    """Add tracks to YouTube Music liked songs."""
    print(f"\n=== Importing {len(tracks)} liked songs ===\n")
    success = 0
    failed = []
    for i, track in enumerate(tracks, 1):
        video_id = search_song(yt, track["name"], track["artist"])
        if video_id:
            try:
                yt.rate_song(video_id, "LIKE")
                print(
                    f"  [{i}/{len(tracks)}] Liked: {track['artist']} - {track['name']}"
                )
                success += 1
            except Exception as e:
                print(f"  [{i}/{len(tracks)}] Error liking '{track['name']}': {e}")
                failed.append(track)
        else:
            print(
                f"  [{i}/{len(tracks)}] Not found: {track['artist']} - {track['name']}"
            )
            failed.append(track)
        time.sleep(0.5)
    return success, failed


def find_playlist(yt, name):
    """Find an existing playlist by name, return playlist ID or None."""
    try:
        playlists = yt.get_library_playlists(limit=None)
        for p in playlists:
            if p.get("title") == name:
                return p["playlistId"]
    except Exception:
        pass
    return None


def get_playlist_video_ids(yt, playlist_id):
    """Get the set of video IDs already in a playlist."""
    try:
        playlist = yt.get_playlist(playlist_id, limit=None)
        return {t["videoId"] for t in playlist.get("tracks", []) if t.get("videoId")}
    except Exception:
        return set()


def import_playlist(yt, playlist_name, tracks):
    """Create or update a playlist with tracks. Returns (added, skipped, failed)."""
    print(f"\n=== Importing playlist '{playlist_name}' ({len(tracks)} songs) ===\n")
    video_ids = []
    failed = []
    for i, track in enumerate(tracks, 1):
        video_id = search_song(yt, track["name"], track["artist"])
        if video_id:
            video_ids.append(video_id)
            print(f"  [{i}/{len(tracks)}] Found: {track['artist']} - {track['name']}")
        else:
            print(
                f"  [{i}/{len(tracks)}] Not found: {track['artist']} - {track['name']}"
            )
            failed.append(track)
        time.sleep(0.5)

    if video_ids:
        try:
            existing_id = find_playlist(yt, playlist_name)
            if existing_id:
                existing_videos = get_playlist_video_ids(yt, existing_id)
                new_ids = [v for v in video_ids if v not in existing_videos]
                skipped = len(video_ids) - len(new_ids)
                if new_ids:
                    yt.add_playlist_items(existing_id, new_ids, duplicates=True)
                return len(new_ids), skipped, failed
            else:
                yt.create_playlist(
                    playlist_name,
                    f"Imported from Spotify ({len(video_ids)} songs)",
                    video_ids=video_ids,
                )
                return len(video_ids), 0, failed
        except Exception as e:
            print(f"\n  Error with playlist: {e}")
            return 0, 0, failed

    return 0, 0, failed


def write_failure_log(all_failed):
    """Write failed imports to a timestamped log file in output/.

    Format:
      {
        "liked_songs": [{"name": "...", "artist": "..."}],
        "playlists": {
          "Playlist Name": [{"name": "...", "artist": "..."}]
        }
      }
    """
    OUTPUT_DIR.mkdir(exist_ok=True)
    timestamp = datetime.now().strftime("%Y-%m-%d_%H%M%S")
    log_path = OUTPUT_DIR / f"failed_{timestamp}.json"
    with open(log_path, "w", encoding="utf-8") as f:
        json.dump(all_failed, f, indent=2, ensure_ascii=False)
    return log_path


def do_import(csv_dir, auth_path):
    """Import Spotify CSV exports into YouTube Music."""
    if not auth_path.exists():
        print(f"Auth file not found: {auth_path}")
        print("Run 'python import_to_ytmusic.py setup' first.")
        sys.exit(1)

    csv_dir = Path(csv_dir)
    if not csv_dir.is_dir():
        print(f"Not a directory: {csv_dir}")
        sys.exit(1)

    csv_files = sorted(csv_dir.glob("*.csv"))
    if not csv_files:
        print(f"No CSV files found in {csv_dir}")
        sys.exit(1)

    print("Connecting to YouTube Music...")
    yt = YTMusic(str(auth_path))

    liked_file = csv_dir / "Liked_Songs.csv"
    has_liked = liked_file.exists()
    playlist_files = [f for f in csv_files if f.name != "Liked_Songs.csv"]

    print(f"\nFound {len(csv_files)} CSV files:")
    if has_liked:
        print(f"  - Liked Songs ({len(load_csv(liked_file))} songs)")
    for f in playlist_files:
        tracks = load_csv(f)
        name = f.stem.replace("_", " ")
        print(f"  - {name} ({len(tracks)} songs)")

    print("\nOptions:")
    print("  1) Import everything (liked songs + all playlists)")
    if has_liked:
        print("  2) Import liked songs only")
    print("  3) Import playlists only")
    choice = input("\nChoice [1/2/3]: ").strip()

    all_failed = {"liked_songs": [], "playlists": {}}

    if choice in ("1", "2") and has_liked:
        tracks = load_csv(liked_file)
        success, failed = import_liked_songs(yt, tracks)
        print(f"\n  Liked songs: {success} added, {len(failed)} failed")
        if failed:
            all_failed["liked_songs"] = failed

    if choice in ("1", "3"):
        for f in playlist_files:
            tracks = load_csv(f)
            if not tracks:
                continue
            name = f.stem.replace("_", " ")
            added, skipped, failed = import_playlist(yt, name, tracks)
            parts = []
            if added:
                parts.append(f"{added} added")
            if skipped:
                parts.append(f"{skipped} skipped")
            if failed:
                parts.append(f"{len(failed)} failed")
            print(f"\n  '{name}': {', '.join(parts) or 'nothing to do'}")
            if failed:
                all_failed["playlists"][name] = failed

    has_failures = all_failed["liked_songs"] or all_failed["playlists"]
    if has_failures:
        log_path = write_failure_log(all_failed)
        print(f"\n\n=== Songs not found (logged to {log_path}) ===")
        if all_failed["liked_songs"]:
            print("\n  Liked Songs:")
            for t in all_failed["liked_songs"]:
                print(f"    - {t['artist']} - {t['name']}")
        for playlist, tracks in all_failed["playlists"].items():
            print(f"\n  {playlist}:")
            for t in tracks:
                print(f"    - {t['artist']} - {t['name']}")
        print(f"\nRetry with: python import_to_ytmusic.py retry {log_path}")

    print("\nDone!")


def do_retry(failure_log, auth_path):
    """Retry failed imports from a previous failure log."""
    if not auth_path.exists():
        print(f"Auth file not found: {auth_path}")
        print("Run 'python import_to_ytmusic.py setup' first.")
        sys.exit(1)

    failure_log = Path(failure_log)
    if not failure_log.exists():
        print(f"File not found: {failure_log}")
        sys.exit(1)

    with open(failure_log, encoding="utf-8") as f:
        data = json.load(f)

    liked = data.get("liked_songs", [])
    playlists = data.get("playlists", {})

    total = len(liked) + sum(len(t) for t in playlists.values())
    if total == 0:
        print("No failed tracks to retry.")
        return

    print(f"Retrying {total} failed tracks from {failure_log.name}...")
    if liked:
        print(f"  - Liked Songs ({len(liked)} songs)")
    for name, tracks in playlists.items():
        print(f"  - {name} ({len(tracks)} songs)")

    print("\nConnecting to YouTube Music...")
    yt = YTMusic(str(auth_path))

    all_failed = {"liked_songs": [], "playlists": {}}

    if liked:
        success, failed = import_liked_songs(yt, liked)
        print(f"\n  Liked songs: {success} added, {len(failed)} still failing")
        if failed:
            all_failed["liked_songs"] = failed

    for name, tracks in playlists.items():
        added, skipped, failed = import_playlist(yt, name, tracks)
        parts = []
        if added:
            parts.append(f"{added} added")
        if skipped:
            parts.append(f"{skipped} skipped")
        if failed:
            parts.append(f"{len(failed)} failed")
        print(f"\n  '{name}': {', '.join(parts) or 'nothing to do'}")
        if failed:
            all_failed["playlists"][name] = failed

    has_failures = all_failed["liked_songs"] or all_failed["playlists"]
    if has_failures:
        log_path = write_failure_log(all_failed)
        print(f"\n\n=== Still failing (logged to {log_path}) ===")
        if all_failed["liked_songs"]:
            print("\n  Liked Songs:")
            for t in all_failed["liked_songs"]:
                print(f"    - {t['artist']} - {t['name']}")
        for playlist, tracks in all_failed["playlists"].items():
            print(f"\n  {playlist}:")
            for t in tracks:
                print(f"    - {t['artist']} - {t['name']}")
        print(f"\nRetry again with: python import_to_ytmusic.py retry {log_path}")
    else:
        print("\nAll tracks imported successfully!")

    print("\nDone!")


def main():
    parser = argparse.ArgumentParser(
        description="Import Spotify CSV exports into YouTube Music",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    # setup
    setup_parser = subparsers.add_parser("setup", help="Set up YouTube Music auth")
    setup_parser.add_argument(
        "--headers-json",
        type=Path,
        default=None,
        help="Path to a JSON file with YouTube Music browser headers",
    )
    setup_parser.add_argument(
        "--auth",
        type=Path,
        default=DEFAULT_AUTH_PATH,
        help=f"Path to save auth file (default: {DEFAULT_AUTH_PATH})",
    )

    # import
    import_parser = subparsers.add_parser("import", help="Import from CSV folder")
    import_parser.add_argument("path", help="Path to folder with Spotify CSV files")
    import_parser.add_argument(
        "--auth",
        type=Path,
        default=DEFAULT_AUTH_PATH,
        help=f"Path to YouTube Music auth JSON (default: {DEFAULT_AUTH_PATH})",
    )

    # retry
    retry_parser = subparsers.add_parser(
        "retry", help="Retry failed imports from a log file"
    )
    retry_parser.add_argument("failure_log", help="Path to a failed_*.json log file")
    retry_parser.add_argument(
        "--auth",
        type=Path,
        default=DEFAULT_AUTH_PATH,
        help=f"Path to YouTube Music auth JSON (default: {DEFAULT_AUTH_PATH})",
    )

    args = parser.parse_args()

    if args.command == "setup":
        do_setup(args.auth, args.headers_json)
    elif args.command == "import":
        do_import(args.path, args.auth)
    elif args.command == "retry":
        do_retry(args.failure_log, args.auth)


if __name__ == "__main__":
    main()
