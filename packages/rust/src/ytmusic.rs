use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::HashMap;

const YTM_BASE_API: &str = "https://music.youtube.com/youtubei/v1/";
const YTM_PARAMS: &str = "?alt=json&key=AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30";

pub struct YTMusic {
    client: Client,
    headers: HashMap<String, String>,
    context: Value,
}

impl YTMusic {
    pub fn new(auth_path: &std::path::Path) -> Self {
        let content =
            std::fs::read_to_string(auth_path).expect("Failed to read YouTube Music auth file");
        let headers: HashMap<String, String> =
            serde_json::from_str(&content).expect("Invalid auth JSON");

        let client_version = format!("1.{}.01.00", Utc::now().format("%Y%m%d"));
        let context = json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": client_version
            },
            "user": {}
        });

        YTMusic {
            client: Client::new(),
            headers,
            context,
        }
    }

    fn get_auth_header(&self) -> String {
        let cookie = self.headers.get("cookie").map(|s| s.as_str()).unwrap_or("");
        let origin = "https://music.youtube.com";

        // Extract SAPISID from cookie
        let sapisid = cookie
            .split(';')
            .find_map(|part| {
                let part = part.trim();
                if part.starts_with("SAPISID=") || part.starts_with("__Secure-3PAPISID=") {
                    part.split_once('=').map(|(_, v)| v.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let timestamp = Utc::now().timestamp();
        let auth_string = format!("{} {} {}", timestamp, sapisid, origin);

        let mut hasher = Sha1::new();
        hasher.update(auth_string.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        format!("SAPISIDHASH {}_{}", timestamp, hash)
    }

    async fn send_request(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}{}{}", YTM_BASE_API, endpoint, YTM_PARAMS);

        let mut request_body = body.clone();
        request_body
            .as_object_mut()
            .unwrap()
            .insert("context".to_string(), self.context.clone());

        let authorization = self.get_auth_header();
        let cookie = self.headers.get("cookie").cloned().unwrap_or_default();

        let resp = self
            .client
            .post(&url)
            .header("authorization", &authorization)
            .header("cookie", &cookie)
            .header("content-type", "application/json")
            .header("origin", "https://music.youtube.com")
            .header("referer", "https://music.youtube.com/")
            .header(
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:88.0) Gecko/20100101 Firefox/88.0",
            )
            .header(
                "x-goog-authuser",
                self.headers.get("x-goog-authuser").unwrap_or(&"0".into()),
            )
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, text));
        }

        resp.json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))
    }

    pub async fn search_song(&self, name: &str, artist: &str) -> Option<String> {
        let query = format!("{} {}", name, artist);
        // EgWKAQIIAWoKEAkQBRAKEAMQBA== is the param for filter=songs
        let body = json!({
            "query": query,
            "params": "EgWKAQIIAWoKEAkQBRAKEAMQBA=="
        });

        let resp = match self.send_request("search", &body).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  Search error for '{}': {}", query, e);
                return None;
            }
        };

        // Navigate: contents.tabbedSearchResultsRenderer.tabs[0].tabRenderer.content
        //   .sectionListRenderer.contents[].musicShelfRenderer.contents[]
        //   .musicResponsiveListItemRenderer.overlay.musicItemThumbnailOverlayRenderer
        //   .content.musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint.videoId
        //
        // Simpler path: look for first videoId in musicShelfRenderer results
        extract_first_video_id(&resp)
    }

    pub async fn rate_song(&self, video_id: &str, rating: &str) -> Result<(), String> {
        let endpoint = match rating {
            "LIKE" => "like/like",
            "DISLIKE" => "like/dislike",
            "INDIFFERENT" => "like/removelike",
            _ => return Err(format!("Invalid rating: {}", rating)),
        };

        let body = json!({
            "target": {
                "videoId": video_id
            }
        });

        self.send_request(endpoint, &body).await?;
        Ok(())
    }

    pub async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> Result<String, String> {
        let body = json!({
            "title": title,
            "description": description,
            "privacyStatus": "PRIVATE",
            "videoIds": video_ids
        });

        let resp = self.send_request("playlist/create", &body).await?;

        resp.get("playlistId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "No playlistId in response".to_string())
    }

    pub async fn find_playlist(&self, name: &str) -> Option<String> {
        let body = json!({
            "browseId": "FEmusic_liked_playlists"
        });

        let resp = match self.send_request("browse", &body).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  Failed to fetch playlists: {}", e);
                return None;
            }
        };

        // Navigate the response to find playlists
        let contents = resp.get("contents")?;
        // Try singleColumnBrowseResultsRenderer (older) or twoColumnBrowseResultsRenderer (newer)
        let tabs = contents
            .get("singleColumnBrowseResultsRenderer")
            .or_else(|| contents.get("twoColumnBrowseResultsRenderer"))
            .and_then(|r| r.get("tabs"))
            .and_then(|t| t.as_array());

        let tabs = tabs?;

        let items = tabs
            .first()?
            .get("tabRenderer")?
            .get("content")?
            .get("sectionListRenderer")?
            .get("contents")?
            .as_array()?;

        for section in items {
            let grid_items = section
                .get("gridRenderer")
                .or_else(|| section.get("musicShelfRenderer"))
                .and_then(|r| r.get("items").or_else(|| r.get("contents")))
                .and_then(|i| i.as_array());

            if let Some(grid_items) = grid_items {
                for item in grid_items {
                    let renderer = item
                        .get("musicTwoRowItemRenderer")
                        .or_else(|| item.get("musicResponsiveListItemRenderer"));

                    if let Some(renderer) = renderer {
                        let title = renderer
                            .get("title")
                            .and_then(|t| t.get("runs"))
                            .and_then(|r| r.as_array())
                            .and_then(|r| r.first())
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.as_str());

                        let playlist_id = renderer
                            .get("navigationEndpoint")
                            .and_then(|n| n.get("browseEndpoint"))
                            .and_then(|b| b.get("browseId"))
                            .and_then(|id| id.as_str())
                            .map(|id| id.strip_prefix("VL").unwrap_or(id));


                        if let (Some(t), Some(id)) = (title, playlist_id) {
                            if t == name {
                                return Some(id.to_string());
                            }
                        }
                    }
                }
            }
        }

        None
    }

    pub async fn get_playlist_video_ids(&self, playlist_id: &str) -> std::collections::HashSet<String> {
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{}", playlist_id)
        };

        let body = json!({ "browseId": browse_id });
        let mut ids = std::collections::HashSet::new();

        let resp = match self.send_request("browse", &body).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  Failed to fetch playlist tracks: {}", e);
                return ids;
            }
        };


        // Extract videoIds from playlist tracks
        if let Some(tracks) = extract_playlist_tracks(&resp) {
            for track in tracks {
                if let Some(vid) = track
                    .get("musicResponsiveListItemRenderer")
                    .and_then(|r| r.get("playlistItemData"))
                    .and_then(|d| d.get("videoId"))
                    .and_then(|v| v.as_str())
                {
                    ids.insert(vid.to_string());
                } else if let Some(vid) = extract_video_id_from_item(track) {
                    ids.insert(vid);
                }
            }
        }

        ids
    }

    pub async fn add_playlist_items(
        &self,
        playlist_id: &str,
        video_ids: &[String],
    ) -> Result<(), String> {
        let actions: Vec<Value> = video_ids
            .iter()
            .map(|id| {
                json!({
                    "action": "ACTION_ADD_VIDEO",
                    "addedVideoId": id,
                    "dedupeOption": "DEDUPE_OPTION_SKIP"
                })
            })
            .collect();

        let body = json!({
            "playlistId": playlist_id,
            "actions": actions
        });

        self.send_request("browse/edit_playlist", &body).await?;
        Ok(())
    }
}

fn extract_playlist_tracks(resp: &Value) -> Option<&Vec<Value>> {
    let contents = resp.get("contents")?;

    // Try twoColumnBrowseResultsRenderer (current format)
    if let Some(tracks) = contents
        .get("twoColumnBrowseResultsRenderer")
        .and_then(|r| r.get("secondaryContents"))
        .and_then(|s| s.get("sectionListRenderer"))
        .and_then(|s| s.get("contents"))
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("musicPlaylistShelfRenderer"))
        .and_then(|r| r.get("contents"))
        .and_then(|c| c.as_array())
    {
        return Some(tracks);
    }

    // Fallback: singleColumnBrowseResultsRenderer (older format)
    contents
        .get("singleColumnBrowseResultsRenderer")?
        .get("tabs")?
        .as_array()?
        .first()?
        .get("tabRenderer")?
        .get("content")?
        .get("sectionListRenderer")?
        .get("contents")?
        .as_array()?
        .first()?
        .get("musicPlaylistShelfRenderer")?
        .get("contents")?
        .as_array()
}

fn extract_first_video_id(resp: &Value) -> Option<String> {
    // Try tabbedSearchResultsRenderer path
    let tabs = resp
        .get("contents")?
        .get("tabbedSearchResultsRenderer")?
        .get("tabs")?
        .as_array()?;

    let section_contents = tabs
        .first()?
        .get("tabRenderer")?
        .get("content")?
        .get("sectionListRenderer")?
        .get("contents")?
        .as_array()?;

    for section in section_contents {
        let items = section
            .get("musicShelfRenderer")
            .and_then(|r| r.get("contents"))
            .and_then(|c| c.as_array());

        if let Some(items) = items {
            for item in items {
                if let Some(video_id) = extract_video_id_from_item(item) {
                    return Some(video_id);
                }
            }
        }
    }

    None
}

fn extract_video_id_from_item(item: &Value) -> Option<String> {
    // Path: musicResponsiveListItemRenderer.overlay.musicItemThumbnailOverlayRenderer
    //   .content.musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint.videoId
    let renderer = item.get("musicResponsiveListItemRenderer")?;

    // Try overlay path
    if let Some(video_id) = renderer
        .get("overlay")
        .and_then(|o| o.get("musicItemThumbnailOverlayRenderer"))
        .and_then(|r| r.get("content"))
        .and_then(|c| c.get("musicPlayButtonRenderer"))
        .and_then(|r| r.get("playNavigationEndpoint"))
        .and_then(|e| e.get("watchEndpoint"))
        .and_then(|w| w.get("videoId"))
        .and_then(|v| v.as_str())
    {
        return Some(video_id.to_string());
    }

    // Try flexColumns path for videoId
    if let Some(video_id) = renderer
        .get("flexColumns")
        .and_then(|fc| fc.as_array())
        .and_then(|fc| fc.first())
        .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
        .and_then(|r| r.get("text"))
        .and_then(|t| t.get("runs"))
        .and_then(|r| r.as_array())
        .and_then(|r| r.first())
        .and_then(|r| r.get("navigationEndpoint"))
        .and_then(|n| n.get("watchEndpoint"))
        .and_then(|w| w.get("videoId"))
        .and_then(|v| v.as_str())
    {
        return Some(video_id.to_string());
    }

    // Try playlistItemData path
    if let Some(video_id) = renderer
        .get("playlistItemData")
        .and_then(|d| d.get("videoId"))
        .and_then(|v| v.as_str())
    {
        return Some(video_id.to_string());
    }

    None
}
