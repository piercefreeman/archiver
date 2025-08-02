mod storage;

use axum::{
    extract::State,
    http::Method,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use storage::{ArchivedRequest, ArchivedResponse, PageFetchIndex, Storage};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, debug};
use tracing_subscriber;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HttpHeader {
    name: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ArchiveEntry {
    Request {
        id: String,
        timestamp: i64,
        url: String,
        method: String,
        request_headers: Option<Vec<HttpHeader>>,
        request_body: Option<serde_json::Value>,
    },
    Response {
        id: String,
        timestamp: i64,
        url: String,
        method: String,
        status_code: Option<u16>,
        response_headers: Option<Vec<HttpHeader>>,
        response_body: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PasswordHash {
    id: String,
    timestamp: i64,
    url: String,
    field: String,
    hash: String,
}

#[derive(Clone)]
struct AppState {
    storage: Arc<Storage>,
    active_sessions: Arc<Mutex<HashMap<String, PageFetchIndex>>>,
    rrweb_sessions: Arc<Mutex<HashMap<String, RrwebSession>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArchiveRequest {
    entries: Vec<ArchiveEntry>,
    password_hashes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PasswordHashRequest {
    hashes: Vec<PasswordHash>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RrwebRecordingRequest {
    session_id: String,
    url: String,
    timestamp: i64,
    events: Vec<serde_json::Value>,
    password_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RrwebSession {
    session_id: String,
    url: String,
    timestamp: i64,
    events: Vec<serde_json::Value>,
    password_hashes: HashSet<String>,
}

#[derive(Debug, Serialize)]
struct ArchiveResponse {
    success: bool,
    message: String,
    count: usize,
}

#[derive(Debug, Serialize)]
struct StatsResponse {
    total_archives: usize,
    total_password_hashes: usize,
    requests: usize,
    responses: usize,
    sessions: usize,
    events: usize,
    storage: storage::StorageStats,
}

async fn health() -> &'static str {
    "OK"
}

fn strip_password_hashes(text: &str, hashes: &HashSet<String>) -> String {
    let mut result = text.to_string();
    for hash in hashes {
        result = result.replace(hash, "[REDACTED]");
    }
    result
}

async fn archive_entries(
    State(state): State<AppState>,
    Json(payload): Json<ArchiveRequest>,
) -> Json<ArchiveResponse> {
    let count = payload.entries.len();
    let password_hashes: HashSet<String> = payload.password_hashes.into_iter().collect();
    
    // Group entries by session/page
    let mut page_requests: HashMap<String, Vec<(ArchiveEntry, Option<ArchiveEntry>)>> = HashMap::new();
    let mut pending_requests: HashMap<String, ArchiveEntry> = HashMap::new();
    
    for entry in payload.entries {
        match &entry {
            ArchiveEntry::Request { id, url, .. } => {
                // Extract session ID from URL or use a default
                let session_id = extract_session_id(url);
                let entry_clone = entry.clone();
                pending_requests.insert(id.clone(), entry_clone.clone());
                
                page_requests.entry(session_id)
                    .or_insert_with(Vec::new)
                    .push((entry_clone, None));
            }
            ArchiveEntry::Response { id, .. } => {
                // Find matching request
                let request_id = id.trim_end_matches("_response");
                if let Some(request) = pending_requests.get(request_id) {
                    let session_id = extract_session_id(&match request {
                        ArchiveEntry::Request { url, .. } => url.clone(),
                        _ => String::new(),
                    });
                    
                    // Update the page requests with the response
                    if let Some(requests) = page_requests.get_mut(&session_id) {
                        for (req, resp) in requests.iter_mut() {
                            if let ArchiveEntry::Request { id: req_id, .. } = req {
                                if req_id == request_id {
                                    *resp = Some(entry.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Process each page's requests
    for (session_id, requests) in page_requests {
        let mut page_fetch = {
            let mut sessions = state.active_sessions.lock().await;
            sessions.entry(session_id.clone())
                .or_insert_with(|| PageFetchIndex {
                    session_id: session_id.clone(),
                    page_url: String::new(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    navigation_id: Uuid::new_v4().to_string(),
                    requests: Vec::new(),
                    password_hashes: password_hashes.iter().cloned().collect(),
                })
                .clone()
        };
        
        // Process each request/response pair
        for (request, response) in requests {
            if let ArchiveEntry::Request { url, method, request_headers, request_body, timestamp, .. } = request {
                // Set page URL if not set
                if page_fetch.page_url.is_empty() {
                    page_fetch.page_url = strip_password_hashes(&url, &password_hashes);
                }
                
                let mut archived_request = ArchivedRequest {
                    request_id: Uuid::new_v4().to_string(),
                    timestamp,
                    method,
                    url: strip_password_hashes(&url, &password_hashes),
                    request_headers: convert_headers(request_headers, &password_hashes),
                    request_body_hash: None,
                    request_body_size: None,
                    response: None,
                };
                
                // Store request body if present
                if let Some(body) = request_body {
                    let body_str = serde_json::to_string(&body).unwrap_or_default();
                    let cleaned_body = strip_password_hashes(&body_str, &password_hashes);
                    let body_bytes = cleaned_body.as_bytes();
                    
                    if !body_bytes.is_empty() {
                        match state.storage.store_content(body_bytes).await {
                            Ok(hash) => {
                                archived_request.request_body_hash = Some(hash);
                                archived_request.request_body_size = Some(body_bytes.len());
                            }
                            Err(e) => {
                                tracing::error!("Failed to store request body: {}", e);
                            }
                        }
                    }
                }
                
                // Process response if present
                if let Some(ArchiveEntry::Response { status_code, response_headers, response_body, .. }) = response {
                    let mut archived_response = ArchivedResponse {
                        status_code: status_code.unwrap_or(0),
                        headers: convert_headers(response_headers, &password_hashes),
                        body_hash: None,
                        body_size: None,
                        body_type: None,
                    };
                    
                    // Detect content type
                    for (name, value) in &archived_response.headers {
                        if name.to_lowercase() == "content-type" {
                            archived_response.body_type = Some(value.clone());
                            break;
                        }
                    }
                    
                    // Store response body if present
                    if let Some(body) = response_body {
                        let cleaned_body = strip_password_hashes(&body, &password_hashes);
                        let body_bytes = cleaned_body.as_bytes();
                        
                        if !body_bytes.is_empty() {
                            match state.storage.store_content(body_bytes).await {
                                Ok(hash) => {
                                    archived_response.body_hash = Some(hash);
                                    archived_response.body_size = Some(body_bytes.len());
                                }
                                Err(e) => {
                                    tracing::error!("Failed to store response body: {}", e);
                                }
                            }
                        }
                    }
                    
                    archived_request.response = Some(archived_response);
                }
                
                page_fetch.requests.push(archived_request);
            }
        }
        
        // Store the page fetch index
        match state.storage.store_page_fetch(&session_id, &page_fetch).await {
            Ok(path) => {
                info!("Stored page fetch at: {:?}", path);
                
                // Update active sessions
                let mut sessions = state.active_sessions.lock().await;
                sessions.insert(session_id, page_fetch);
            }
            Err(e) => {
                tracing::error!("Failed to store page fetch: {}", e);
            }
        }
    }
    
    Json(ArchiveResponse {
        success: true,
        message: format!("Archived {} entries", count),
        count,
    })
}

fn convert_headers(headers: Option<Vec<HttpHeader>>, password_hashes: &HashSet<String>) -> Vec<(String, String)> {
    headers.map(|h| {
        h.into_iter()
            .map(|header| (header.name, strip_password_hashes(&header.value, password_hashes)))
            .collect()
    }).unwrap_or_default()
}

fn extract_session_id(url: &str) -> String {
    // Simple session ID extraction - in production, this would be more sophisticated
    // For now, use the domain as session ID
    url.split('/')
        .nth(2)
        .unwrap_or("default")
        .to_string()
}

async fn archive_passwords(
    State(_state): State<AppState>,
    Json(payload): Json<PasswordHashRequest>,
) -> Json<ArchiveResponse> {
    let count = payload.hashes.len();
    
    for hash in payload.hashes {
        info!("Password hash recorded for field '{}' on {}", hash.field, hash.url);
    }
    
    Json(ArchiveResponse {
        success: true,
        message: format!("Recorded {} password hashes", count),
        count,
    })
}

async fn archive_recording(
    State(state): State<AppState>,
    Json(payload): Json<RrwebRecordingRequest>,
) -> Json<ArchiveResponse> {
    info!("📹 Received recording request for session: {} from URL: {}", 
        payload.session_id, payload.url);
    
    let event_count = payload.events.len();
    debug!("Event batch size: {}", event_count);
    
    let mut sessions = state.rrweb_sessions.lock().await;
    let _is_new_session = !sessions.contains_key(&payload.session_id);
    
    let session = sessions.entry(payload.session_id.clone())
        .or_insert_with(|| {
            info!("🆕 Creating new recording session: {}", payload.session_id);
            RrwebSession {
                session_id: payload.session_id.clone(),
                url: payload.url.clone(),
                timestamp: payload.timestamp,
                events: Vec::new(),
                password_hashes: HashSet::new(),
            }
        });
    
    // Add events to session
    session.events.extend(payload.events);
    
    // Add password hashes
    let new_hashes = payload.password_hashes.len();
    for hash in payload.password_hashes {
        session.password_hashes.insert(hash);
    }
    
    info!("✅ Recording session {} updated: {} new events, {} new password hashes, {} total events", 
        payload.session_id, event_count, new_hashes, session.events.len());
    
    // Log first few event types for debugging
    if event_count > 0 {
        debug!("Event types in batch: {:?}", 
            session.events.iter()
                .rev()
                .take(3)
                .map(|e| e.get("type").and_then(|t| t.as_u64()))
                .collect::<Vec<_>>()
        );
    }
    
    // TODO: In the future, this is where we would:
    // 1. Check if we have enough events to replay
    // 2. Spawn a headless browser to replay the session
    // 3. Capture all network requests during replay
    // 4. Store the captured data
    
    Json(ArchiveResponse {
        success: true,
        message: format!("Received {} events for recording session", event_count),
        count: event_count,
    })
}

async fn get_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    debug!("📊 Stats request received");
    
    let storage_stats = state.storage.get_stats().await
        .unwrap_or(storage::StorageStats {
            content_count: 0,
            cache_size: 0,
            total_size: 0,
            compressed_size: 0,
            compression_ratio: 1.0,
        });
    
    let sessions = state.active_sessions.lock().await;
    let mut total_requests = 0;
    let mut total_responses = 0;
    
    for (_, page_fetch) in sessions.iter() {
        total_requests += page_fetch.requests.len();
        total_responses += page_fetch.requests.iter()
            .filter(|r| r.response.is_some())
            .count();
    }
    
    // Get rrweb session stats
    let rrweb_sessions = state.rrweb_sessions.lock().await;
    let rrweb_session_count = rrweb_sessions.len();
    let total_events: usize = rrweb_sessions.values()
        .map(|s| s.events.len())
        .sum();
    
    let stats = StatsResponse {
        total_archives: total_requests,
        total_password_hashes: 0, // We don't store these separately anymore
        requests: total_requests,
        responses: total_responses,
        sessions: rrweb_session_count,
        events: total_events,
        storage: storage_stats,
    };
    
    debug!("📊 Stats: {} sessions, {} events, {} requests", 
        rrweb_session_count, total_events, total_requests);
    
    Json(stats)
}

#[tokio::main]
async fn main() {
    // Initialize tracing with environment filter
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();
    
    // Initialize storage
    let storage = Storage::new("./archiver-data").await
        .expect("Failed to initialize storage");
    
    let state = AppState {
        storage: Arc::new(storage),
        active_sessions: Arc::new(Mutex::new(HashMap::new())),
        rrweb_sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);
    
    let app = Router::new()
        .route("/health", get(health))
        .route("/archive", post(archive_entries))
        .route("/passwords", post(archive_passwords))
        .route("/recording", post(archive_recording))
        .route("/stats", get(get_stats))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());
    
    let listener = tokio::net::TcpListener::bind("127.0.0.1:41788")
        .await
        .unwrap();
    
    info!("🚀 Archiver server listening on http://127.0.0.1:41788");
    info!("💾 Storage initialized at ./archiver-data");
    info!("📝 Logging level: {}", std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()));
    info!("Ready to receive rrweb recordings!");
    
    axum::serve(listener, app).await.unwrap();
}