import { record } from 'rrweb';

console.log('🎬 Web Archiver content script loaded!');

interface RecordingSession {
  id: string;
  url: string;
  timestamp: number;
  events: any[];
}

let currentSession: RecordingSession | null = null;
let stopRecording: any = null;
const BATCH_SIZE = 10; // Send events in batches (reduced for testing)
const BATCH_INTERVAL = 2000; // Send every 2 seconds

function generateSessionId(): string {
  return `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
}

async function sha256(text: string): Promise<string> {
  const msgUint8 = new TextEncoder().encode(text);
  const hashBuffer = await crypto.subtle.digest('SHA-256', msgUint8);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  const hashHex = hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
  return hashHex;
}

function startNewSession() {
  console.log('🎬 Starting new recording session for:', window.location.href);
  
  // Stop any existing recording
  if (stopRecording) {
    stopRecording();
  }
  
  currentSession = {
    id: generateSessionId(),
    url: window.location.href,
    timestamp: Date.now(),
    events: []
  };
  
  console.log('📹 Recording session ID:', currentSession.id);
  
  // Start recording with rrweb
  stopRecording = record({
    emit(event) {
      if (currentSession) {
        currentSession.events.push(event);
        console.log(`🎯 Recorded event type ${event.type}, total events: ${currentSession.events.length}`);
        
        // Send batch if we've collected enough events
        if (currentSession.events.length >= BATCH_SIZE) {
          console.log('📦 Batch size reached, sending events...');
          sendEventBatch();
        }
      }
    },
    // Capture everything for comprehensive replay
    checkoutEveryNms: 5000, // Take full snapshot every 5 seconds
    recordCanvas: true,
    recordCrossOriginIframes: false, // Can't access cross-origin iframes
    inlineStylesheet: true,
    collectFonts: true,
    // Mask sensitive data
    maskInputOptions: {
      password: true,
      email: true,
      tel: true
    },
    // Privacy options
    maskAllInputs: false,
    // Performance options
    sampling: {
      scroll: 150, // Sample scroll events
      input: 'last', // Only record last input in a burst
      media: 800,
    },
    // Record user interactions are enabled by default
  });
}

function sendEventBatch() {
  if (!currentSession || currentSession.events.length === 0) return;
  
  const batch = currentSession.events.splice(0, BATCH_SIZE);
  console.log(`📤 Sending ${batch.length} events to background script`);
  
  chrome.runtime.sendMessage({
    type: 'RRWEB_EVENTS',
    sessionId: currentSession.id,
    url: currentSession.url,
    timestamp: currentSession.timestamp,
    events: batch
  }, (response) => {
    if (chrome.runtime.lastError) {
      console.error('❌ Error sending events:', chrome.runtime.lastError);
    } else {
      console.log('✅ Events sent successfully');
    }
  });
}

// Handle password fields separately for hashing
async function handlePasswordInput(event: Event) {
  const target = event.target as HTMLInputElement;
  
  if (target.type === 'password' && target.value) {
    const hashedPassword = await sha256(target.value);
    
    chrome.runtime.sendMessage({
      type: 'PASSWORD_HASH',
      url: window.location.href,
      field: target.name || target.id || 'unnamed',
      hash: hashedPassword,
      timestamp: Date.now()
    });
    
    console.log('Password field detected and hashed');
  }
}

// Listen for password inputs
function attachPasswordListeners() {
  document.addEventListener('blur', async (event) => {
    if ((event.target as HTMLElement).tagName === 'INPUT') {
      await handlePasswordInput(event);
    }
  }, true);
  
  // Watch for dynamically added password fields
  const observer = new MutationObserver((mutations) => {
    mutations.forEach((mutation) => {
      mutation.addedNodes.forEach((node) => {
        if (node.nodeType === 1) {
          const element = node as Element;
          const inputs = element.querySelectorAll('input[type="password"]');
          inputs.forEach((input) => {
            input.addEventListener('blur', handlePasswordInput);
          });
        }
      });
    });
  });
  
  // Wait for body to exist before observing
  if (document.body) {
    observer.observe(document.body, {
      childList: true,
      subtree: true
    });
  } else {
    // If body doesn't exist yet, wait for it
    const bodyObserver = new MutationObserver(() => {
      if (document.body) {
        observer.observe(document.body, {
          childList: true,
          subtree: true
        });
        bodyObserver.disconnect();
      }
    });
    bodyObserver.observe(document.documentElement, {
      childList: true,
      subtree: true
    });
  }
}

// Send remaining events periodically
setInterval(() => {
  sendEventBatch();
}, BATCH_INTERVAL);

// Handle page visibility changes
document.addEventListener('visibilitychange', () => {
  if (document.hidden) {
    // Send all pending events when page is hidden
    sendEventBatch();
  }
});

// Handle page unload
window.addEventListener('beforeunload', () => {
  sendEventBatch();
});

// Handle navigation within single-page apps
function setupUrlObserver() {
  let lastUrl = window.location.href;
  const urlObserver = new MutationObserver(() => {
    if (window.location.href !== lastUrl) {
      lastUrl = window.location.href;
      // Send remaining events from previous session
      sendEventBatch();
      // Start new session for new URL
      startNewSession();
    }
  });

  if (document.body) {
    urlObserver.observe(document.body, {
      childList: true,
      subtree: true
    });
  } else {
    // Wait for body before starting URL observer
    const bodyWatcher = new MutationObserver(() => {
      if (document.body) {
        urlObserver.observe(document.body, {
          childList: true,
          subtree: true
        });
        bodyWatcher.disconnect();
      }
    });
    bodyWatcher.observe(document.documentElement, {
      childList: true,
      subtree: true
    });
  }
}

// Initialize when DOM is ready
console.log('🔧 Initializing Web Archiver, document state:', document.readyState);

// Set up URL observer early
setupUrlObserver();

if (document.readyState === 'loading') {
  console.log('⏳ Waiting for DOMContentLoaded...');
  document.addEventListener('DOMContentLoaded', () => {
    console.log('✅ DOM loaded, starting recording');
    startNewSession();
    attachPasswordListeners();
  });
} else {
  console.log('✅ DOM already loaded, starting recording immediately');
  startNewSession();
  attachPasswordListeners();
}