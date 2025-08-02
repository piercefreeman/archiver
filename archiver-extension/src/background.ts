interface RrwebSession {
  id: string;
  url: string;
  timestamp: number;
  events: any[];
}

interface PasswordEntry {
  id: string;
  timestamp: number;
  type: 'password_hash';
  url: string;
  field: string;
  hash: string;
}

const SERVER_URL = 'http://127.0.0.1:41788';
const BATCH_SIZE = 10; // Smaller batches for testing
const BATCH_INTERVAL = 10000; // 10 seconds

// Store recording sessions
const recordingSessions: Map<string, RrwebSession> = new Map();
let pendingPasswords: PasswordEntry[] = [];
let sessionPasswordHashes: Set<string> = new Set();

function generateId(): string {
  return `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
}

async function sendRecordingSession(sessionId: string) {
  const session = recordingSessions.get(sessionId);
  if (!session || session.events.length === 0) return;
  
  console.log(`📤 Sending ${session.events.length} events to server for session ${sessionId}`);
  
  try {
    const response = await fetch(`${SERVER_URL}/recording`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        session_id: session.id,
        url: session.url,
        timestamp: session.timestamp,
        events: session.events,
        password_hashes: Array.from(sessionPasswordHashes)
      }),
    });
    
    if (response.ok) {
      const result = await response.json();
      console.log('✅ Server response:', result.message);
      // Clear the sent events
      session.events = [];
    } else {
      console.error('❌ Failed to send recording to server:', response.status);
    }
  } catch (error) {
    console.error('❌ Error sending recording to server:', error);
  }
}

async function sendPasswordsToServer() {
  if (pendingPasswords.length === 0) return;
  
  const batch = pendingPasswords.splice(0, BATCH_SIZE);
  
  try {
    const response = await fetch(`${SERVER_URL}/passwords`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ hashes: batch }),
    });
    
    if (response.ok) {
      const result = await response.json();
      console.log('Sent passwords to server:', result.message);
    } else {
      console.error('Failed to send passwords to server:', response.status);
      // Put back in pending queue
      pendingPasswords.unshift(...batch);
    }
  } catch (error) {
    console.error('Error sending passwords to server:', error);
    // Put back in pending queue
    pendingPasswords.unshift(...batch);
  }
}

// Log when background script starts
console.log('🚀 Web Archiver background script loaded');

// Message handling from content scripts
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  console.log('📨 Received message:', message.type, 'from:', sender.tab?.url);
  
  if (message.type === 'RRWEB_EVENTS') {
    // Get or create session
    let session = recordingSessions.get(message.sessionId);
    if (!session) {
      session = {
        id: message.sessionId,
        url: message.url,
        timestamp: message.timestamp,
        events: []
      };
      recordingSessions.set(message.sessionId, session);
    }
    
    // Add events to session
    session.events.push(...message.events);
    console.log(`✅ Added ${message.events.length} events to session ${message.sessionId}, total: ${session.events.length}`);
    
    // Send if we have enough events
    if (session.events.length >= BATCH_SIZE) {
      sendRecordingSession(message.sessionId);
    }
    
    // Send response to content script
    sendResponse({ success: true });
    return true; // Keep message channel open for async response
  } else if (message.type === 'PASSWORD_HASH') {
    const passwordEntry: PasswordEntry = {
      id: generateId(),
      timestamp: message.timestamp,
      type: 'password_hash',
      url: message.url,
      field: message.field,
      hash: message.hash
    };
    
    chrome.storage.local.set({ [`password_${passwordEntry.id}`]: passwordEntry });
    console.log('Password hash saved for field:', message.field);
    
    // Add to session hashes
    sessionPasswordHashes.add(message.hash);
    
    // Add to pending batch
    pendingPasswords.push(passwordEntry);
    
    // Send to server if batch is full
    if (pendingPasswords.length >= BATCH_SIZE) {
      sendPasswordsToServer();
    }
    
    // Send response to content script
    sendResponse({ success: true });
    return true;
  }
});

// Set up periodic batch sending using chrome.alarms for V3 compatibility
chrome.alarms.create('sendBatches', { periodInMinutes: 0.167 }); // ~10 seconds

chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name === 'sendBatches') {
    console.log('⏰ Alarm fired: checking for pending batches...');
    
    // Send all pending recording sessions
    for (const sessionId of recordingSessions.keys()) {
      await sendRecordingSession(sessionId);
    }
    
    // Send pending passwords
    await sendPasswordsToServer();
    
    // Clean up old sessions (older than 1 hour)
    const oneHourAgo = Date.now() - 3600000;
    for (const [sessionId, session] of recordingSessions.entries()) {
      if (session.timestamp < oneHourAgo && session.events.length === 0) {
        recordingSessions.delete(sessionId);
      }
    }
  }
});

// Handle tab close/navigation - send remaining events
chrome.tabs.onRemoved.addListener((tabId) => {
  // Send all pending sessions when a tab is closed
  for (const sessionId of recordingSessions.keys()) {
    sendRecordingSession(sessionId);
  }
});

// Clean up on startup
chrome.runtime.onStartup.addListener(() => {
  console.log('Web Archiver extension started - using rrweb recording');
  recordingSessions.clear();
});