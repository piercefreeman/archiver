# Archiver Storage Design

## Overview
Content-addressed storage system with deduplication for web archive data.

## Directory Structure
```
archiver-data/
├── sessions/
│   └── {date}/
│       └── {session_id}/
│           └── {timestamp}_{page_hash}.json  # Page fetch index
├── content/
│   └── {hash[0:2]}/
│       └── {hash[2:4]}/
│           └── {full_hash}.zst  # Compressed content
├── metadata/
│   └── content_index.db  # sled database for lookups
└── cache/
    └── bloom_filter.bin  # Quick existence checks
```

## Data Flow

1. **Page Load Event**:
   - Create new page fetch index
   - Track all subsequent requests for this page

2. **Request/Response Archiving**:
   - Hash the content (SHA256)
   - Check bloom filter for quick "not exists"
   - Check content index for existence
   - If new: compress and store in content/
   - Reference hash in page fetch index

3. **Deduplication**:
   - Content stored once, referenced many times
   - Headers stored inline (usually unique)
   - Bodies stored by content hash

## Page Fetch Index Format
```json
{
  "session_id": "uuid",
  "page_url": "https://example.com",
  "timestamp": 1234567890,
  "navigation_id": "uuid",
  "requests": [
    {
      "request_id": "uuid",
      "timestamp": 1234567890,
      "method": "GET",
      "url": "https://example.com/api",
      "request_headers": [...],
      "request_body_hash": "sha256:abc123...",
      "request_body_size": 1024,
      "response": {
        "status_code": 200,
        "headers": [...],
        "body_hash": "sha256:def456...",
        "body_size": 2048,
        "body_type": "application/json"
      }
    }
  ],
  "password_hashes": ["sha256:...", "sha256:..."]
}
```

## Content Storage
- Files named by their SHA256 hash
- Compressed with zstd level 3 (balanced speed/ratio)
- Stored in nested directories to avoid filesystem limits
- Example: hash "abc123..." stored at "content/ab/c1/abc123...zst"

## Metadata Index (sled)
- Key-value store for fast lookups
- Tables:
  - `content:{hash}` -> `{size, type, compression, refs}`
  - `session:{id}` -> `[page_fetch_paths]`
  - `url:{hash}` -> `[session_ids]`

## Optimization Strategies
1. Bloom filter for non-existence checks (saves disk I/O)
2. LRU memory cache for frequently accessed content
3. Async I/O for all operations
4. Batch writes to reduce syscalls
5. Periodic compaction of old sessions