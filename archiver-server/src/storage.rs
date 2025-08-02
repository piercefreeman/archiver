use bloomfilter::Bloom;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use zstd::stream::{decode_all, encode_all};

const BLOOM_ITEMS: usize = 1_000_000;
const BLOOM_FP_RATE: f64 = 0.01;
const CACHE_SIZE: usize = 1000;
const COMPRESSION_LEVEL: i32 = 3;

#[derive(Debug, Serialize, Deserialize)]
pub struct ContentMetadata {
    pub size: usize,
    pub compressed_size: usize,
    pub content_type: Option<String>,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub reference_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageFetchIndex {
    pub session_id: String,
    pub page_url: String,
    pub timestamp: i64,
    pub navigation_id: String,
    pub requests: Vec<ArchivedRequest>,
    pub password_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedRequest {
    pub request_id: String,
    pub timestamp: i64,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body_hash: Option<String>,
    pub request_body_size: Option<usize>,
    pub response: Option<ArchivedResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body_hash: Option<String>,
    pub body_size: Option<usize>,
    pub body_type: Option<String>,
}

pub type StorageError = Box<dyn std::error::Error + Send + Sync>;

pub struct Storage {
    base_path: PathBuf,
    content_db: Arc<sled::Db>,
    bloom_filter: Arc<tokio::sync::RwLock<Bloom<String>>>,
    content_cache: Arc<DashMap<String, Vec<u8>>>,
}

impl Storage {
    pub async fn new(base_path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let base_path = base_path.as_ref().to_path_buf();
        
        // Create directory structure
        fs::create_dir_all(&base_path).await?;
        fs::create_dir_all(base_path.join("sessions")).await?;
        fs::create_dir_all(base_path.join("content")).await?;
        fs::create_dir_all(base_path.join("metadata")).await?;
        fs::create_dir_all(base_path.join("cache")).await?;
        
        // Open sled database
        let db_path = base_path.join("metadata").join("content_index.db");
        let content_db = sled::open(&db_path)?;
        
        // Load or create bloom filter
        let bloom = Self::load_or_create_bloom(&base_path).await?;
        
        Ok(Storage {
            base_path,
            content_db: Arc::new(content_db),
            bloom_filter: Arc::new(tokio::sync::RwLock::new(bloom)),
            content_cache: Arc::new(DashMap::new()),
        })
    }
    
    async fn load_or_create_bloom(base_path: &Path) -> Result<Bloom<String>, StorageError> {
        let bloom_path = base_path.join("cache").join("bloom_filter.bin");
        
        if bloom_path.exists() {
            // TODO: Implement bloom filter persistence
            Ok(Bloom::new_for_fp_rate(BLOOM_ITEMS, BLOOM_FP_RATE))
        } else {
            Ok(Bloom::new_for_fp_rate(BLOOM_ITEMS, BLOOM_FP_RATE))
        }
    }
    
    pub fn compute_hash(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("sha256:{}", hex::encode(hasher.finalize()))
    }
    
    pub async fn store_content(&self, data: &[u8]) -> Result<String, StorageError> {
        let hash = Self::compute_hash(data);
        let hash_only = hash.strip_prefix("sha256:").unwrap();
        
        // Check bloom filter first
        {
            let bloom = self.bloom_filter.read().await;
            if bloom.check(&hash) {
                // Might exist, check database
                if self.content_db.contains_key(&hash)? {
                    // Already exists, increment reference count
                    self.increment_ref_count(&hash).await?;
                    return Ok(hash);
                }
            }
        }
        
        // Compress the content
        let compressed = encode_all(data, COMPRESSION_LEVEL)?;
        
        // Store to disk
        let content_path = self.get_content_path(hash_only);
        fs::create_dir_all(content_path.parent().unwrap()).await?;
        
        let mut file = fs::File::create(&content_path).await?;
        file.write_all(&compressed).await?;
        file.sync_all().await?;
        
        // Update metadata
        let metadata = ContentMetadata {
            size: data.len(),
            compressed_size: compressed.len(),
            content_type: None,
            first_seen: chrono::Utc::now(),
            reference_count: 1,
        };
        
        self.content_db.insert(
            hash.as_bytes(),
            serde_json::to_vec(&metadata)?
        )?;
        
        // Update bloom filter
        {
            let mut bloom = self.bloom_filter.write().await;
            bloom.set(&hash);
        }
        
        // Cache if small enough
        if data.len() < 1_000_000 {
            self.content_cache.insert(hash.clone(), data.to_vec());
            
            // Evict old entries if cache is too large
            if self.content_cache.len() > CACHE_SIZE {
                // Simple random eviction
                if let Some(entry) = self.content_cache.iter().next() {
                    self.content_cache.remove(entry.key());
                }
            }
        }
        
        Ok(hash)
    }
    
    pub async fn retrieve_content(&self, hash: &str) -> Result<Vec<u8>, StorageError> {
        // Check cache first
        if let Some(cached) = self.content_cache.get(hash) {
            return Ok(cached.clone());
        }
        
        let hash_only = hash.strip_prefix("sha256:").unwrap_or(hash);
        let content_path = self.get_content_path(hash_only);
        
        if !content_path.exists() {
            return Err("Content not found".into());
        }
        
        let compressed = fs::read(&content_path).await?;
        let decompressed = decode_all(&compressed[..])?;
        
        // Cache if small enough
        if decompressed.len() < 1_000_000 {
            self.content_cache.insert(hash.to_string(), decompressed.clone());
        }
        
        Ok(decompressed)
    }
    
    pub async fn store_page_fetch(&self, session_id: &str, page_fetch: &PageFetchIndex) -> Result<PathBuf, StorageError> {
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let page_hash = Self::compute_hash(page_fetch.page_url.as_bytes());
        let page_hash_only = page_hash.strip_prefix("sha256:").unwrap();
        
        let filename = format!("{}_{}.json", page_fetch.timestamp, &page_hash_only[..8]);
        let path = self.base_path
            .join("sessions")
            .join(&date)
            .join(session_id)
            .join(&filename);
        
        fs::create_dir_all(path.parent().unwrap()).await?;
        
        let json = serde_json::to_string_pretty(page_fetch)?;
        fs::write(&path, json).await?;
        
        // Update session index
        let session_key = format!("session:{}", session_id);
        let mut paths = Vec::new();
        
        if let Ok(Some(existing)) = self.content_db.get(&session_key) {
            paths = serde_json::from_slice(&existing)?;
        }
        
        paths.push(path.to_string_lossy().to_string());
        self.content_db.insert(
            session_key.as_bytes(),
            serde_json::to_vec(&paths)?
        )?;
        
        Ok(path)
    }
    
    async fn increment_ref_count(&self, hash: &str) -> Result<(), StorageError> {
        if let Ok(Some(data)) = self.content_db.get(hash) {
            let mut metadata: ContentMetadata = serde_json::from_slice(&data)?;
            metadata.reference_count += 1;
            self.content_db.insert(
                hash.as_bytes(),
                serde_json::to_vec(&metadata)?
            )?;
        }
        Ok(())
    }
    
    fn get_content_path(&self, hash: &str) -> PathBuf {
        // Split hash for directory structure
        let dir1 = &hash[..2];
        let dir2 = &hash[2..4];
        
        self.base_path
            .join("content")
            .join(dir1)
            .join(dir2)
            .join(format!("{}.zst", hash))
    }
    
    pub async fn get_stats(&self) -> Result<StorageStats, StorageError> {
        let content_count = self.content_db.len();
        let cache_size = self.content_cache.len();
        
        // Calculate total size by iterating metadata
        let mut total_size = 0u64;
        let mut compressed_size = 0u64;
        
        for item in self.content_db.iter() {
            if let Ok((_, value)) = item {
                if let Ok(metadata) = serde_json::from_slice::<ContentMetadata>(&value) {
                    total_size += metadata.size as u64;
                    compressed_size += metadata.compressed_size as u64;
                }
            }
        }
        
        Ok(StorageStats {
            content_count,
            cache_size,
            total_size,
            compressed_size,
            compression_ratio: if total_size > 0 {
                compressed_size as f64 / total_size as f64
            } else {
                1.0
            },
        })
    }
}

#[derive(Debug, Serialize)]
pub struct StorageStats {
    pub content_count: usize,
    pub cache_size: usize,
    pub total_size: u64,
    pub compressed_size: u64,
    pub compression_ratio: f64,
}