import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";

const Popup = () => {
  const [stats, setStats] = useState<any>(null);
  const [serverStatus, setServerStatus] = useState<'checking' | 'online' | 'offline'>('checking');
  const [recordingActive, setRecordingActive] = useState(true);

  useEffect(() => {
    checkServerStatus();
    const interval = setInterval(() => {
      checkServerStatus();
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const checkServerStatus = async () => {
    try {
      const response = await fetch('http://127.0.0.1:41788/stats');
      if (response.ok) {
        const data = await response.json();
        setStats(data);
        setServerStatus('online');
      } else {
        setServerStatus('offline');
      }
    } catch (error) {
      setServerStatus('offline');
    }
  };

  const formatBytes = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  };

  return (
    <div style={{ padding: '16px', minWidth: '350px' }}>
      <h2 style={{ margin: '0 0 16px 0', fontSize: '18px' }}>Web Archiver</h2>
      
      <div style={{ marginBottom: '16px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
          <div style={{
            width: '8px',
            height: '8px',
            borderRadius: '50%',
            backgroundColor: serverStatus === 'online' ? '#4CAF50' : 
                           serverStatus === 'offline' ? '#f44336' : '#FFC107'
          }} />
          <span style={{ fontSize: '14px' }}>
            Server: {serverStatus === 'checking' ? 'Checking...' : serverStatus}
          </span>
        </div>
        
        {serverStatus === 'offline' && (
          <div style={{ 
            fontSize: '12px', 
            color: '#666',
            backgroundColor: '#f5f5f5',
            padding: '8px',
            borderRadius: '4px',
            marginTop: '8px'
          }}>
            Start the server with:<br />
            <code style={{ fontFamily: 'monospace' }}>cd ../archiver-server && cargo run</code>
          </div>
        )}
      </div>

      <div style={{ marginBottom: '16px' }}>
        <h3 style={{ margin: '0 0 8px 0', fontSize: '16px' }}>Recording Status</h3>
        <div style={{ 
          backgroundColor: '#e8f5e9',
          padding: '12px',
          borderRadius: '4px',
          border: '1px solid #4CAF50'
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <div style={{
              width: '8px',
              height: '8px',
              borderRadius: '50%',
              backgroundColor: '#4CAF50',
              animation: 'pulse 2s infinite'
            }} />
            <div style={{ fontWeight: 'bold' }}>Recording Active</div>
          </div>
          <div style={{ fontSize: '12px', color: '#666', marginTop: '4px' }}>
            Capturing page interactions and DOM changes
          </div>
        </div>
        <div style={{ 
          marginTop: '8px', 
          fontSize: '11px', 
          color: '#666',
          backgroundColor: '#f5f5f5',
          padding: '6px',
          borderRadius: '2px'
        }}>
          ℹ️ Pages will be replayed server-side to capture all network requests
        </div>
      </div>

      {stats && serverStatus === 'online' && (
        <div style={{ fontSize: '14px' }}>
          <h3 style={{ margin: '0 0 8px 0', fontSize: '16px' }}>Statistics</h3>
          <div style={{ display: 'grid', gap: '4px' }}>
            <div>Recording Sessions: {stats.sessions || 0}</div>
            <div>Total Events: {stats.events || 0}</div>
            <div>Password Hashes: {stats.password_hashes || 0}</div>
            {stats.storage && (
              <>
                <div>Storage Size: {formatBytes(stats.storage.total_size)}</div>
                <div>Compressed: {formatBytes(stats.storage.compressed_size)}</div>
                <div>Compression: {(stats.storage.compression_ratio * 100).toFixed(1)}%</div>
              </>
            )}
          </div>
        </div>
      )}

      <div style={{ 
        marginTop: '16px', 
        paddingTop: '16px', 
        borderTop: '1px solid #e0e0e0',
        fontSize: '12px',
        color: '#666'
      }}>
        Archiving to: http://127.0.0.1:41788
      </div>

      <style dangerouslySetInnerHTML={{__html: `
        @keyframes pulse {
          0% { opacity: 1; }
          50% { opacity: 0.5; }
          100% { opacity: 1; }
        }
      `}} />
    </div>
  );
};

const root = createRoot(document.getElementById("root")!);

root.render(
  <React.StrictMode>
    <Popup />
  </React.StrictMode>
);