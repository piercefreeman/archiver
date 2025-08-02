# Web Archiver System

A Chrome extension and Rust server for archiving web traffic with password protection.

## Architecture

### Chrome Extension
- Intercepts all HTTP requests and responses
- Captures response bodies via fetch() and XMLHttpRequest hooks
- Detects password fields and computes SHA256 hashes
- Sends data to local server in batches

### Rust Server (Port 41788)
- Content-addressed storage with SHA256 hashing
- Automatic deduplication of identical content
- Zstd compression for efficient storage
- Password hash stripping from all archived data
- File-based storage structure:
  ```
  archiver-data/
  ├── sessions/      # Page fetch indices by date/session
  ├── content/       # Deduplicated content by hash
  ├── metadata/      # sled database for lookups
  └── cache/         # Bloom filters and caches
  ```

## Features

1. **Zero Duplication**: Content stored once, referenced many times
2. **Password Protection**: SHA256 hashes strip credentials from archives
3. **Efficient Storage**: Zstd compression reduces storage by ~50-70%
4. **Fast Lookups**: Bloom filters and LRU cache for performance
5. **Session Tracking**: Groups requests by page navigation

## Setup

### Prerequisites
- Node.js and npm
- Rust and Cargo

### Installation

1. Build the Chrome extension:
   ```bash
   cd archiver-extension
   npm install
   npm run build
   ```

2. Build the Rust server:
   ```bash
   cd archiver-server
   cargo build --release
   ```

## Usage

1. Start the server:
   ```bash
   cd archiver-server
   cargo run
   ```

2. Install the extension:
   - Open Chrome → Extensions → Developer mode
   - Load unpacked → Select `archiver-extension/dist`

3. Browse normally - all traffic is archived automatically

## API Endpoints

- `POST /archive` - Submit archived entries
- `GET /stats` - View storage statistics
- `POST /passwords` - Submit password hashes
- `GET /health` - Health check

## Security

- Passwords are never stored in plain text
- SHA256 hashes are stripped from all archived content
- All data stored locally on your machine

## Development

### Chrome Extension
```bash
cd archiver-extension
npm run watch  # Auto-rebuild on changes
```

### Rust Server
```bash
cd archiver-server
cargo watch -x run  # Auto-restart on changes
```