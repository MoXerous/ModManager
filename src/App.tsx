import { useEffect, useState } from 'react';

import type { AppInfo } from './bindings/AppInfo';
import type { DownloadEvent } from './bindings/DownloadEvent';
import { getAppInfo, startDemoDownload, toIpcError } from './lib/ipc';

type Status = 'idle' | 'running' | 'done' | 'error';

// شاشة تشخيص مؤقتة تثبت أن IPC والأنواع المولَّدة وChannel تعمل. تُستبدل في M2.
export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [status, setStatus] = useState<Status>('idle');
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAppInfo()
      .then(setInfo)
      .catch((e) => setError(toIpcError(e).message));
  }, []);

  const handleEvent = (event: DownloadEvent) => {
    switch (event.event) {
      case 'started':
        setTotal(event.data.total);
        setDownloaded(0);
        break;
      case 'advanced':
        setDownloaded(event.data.downloaded);
        break;
      case 'finished':
        setStatus('done');
        break;
    }
  };

  const run = async () => {
    setStatus('running');
    setError(null);
    try {
      await startDemoDownload(handleEvent);
    } catch (e) {
      setError(toIpcError(e).message);
      setStatus('error');
    }
  };

  return (
    <main style={{ fontFamily: 'system-ui, sans-serif', padding: 24, maxWidth: 480 }}>
      <h1>{info ? info.name : 'ModManager'}</h1>
      {info && (
        <p>
          Version {info.version} on {info.os} ({info.arch})
        </p>
      )}

      <button onClick={run} disabled={status === 'running'}>
        Run demo download
      </button>

      <p>
        <progress value={downloaded} max={total || 1} style={{ width: '100%' }} />
        <br />
        {downloaded.toLocaleString()} / {total.toLocaleString()} bytes ({status})
      </p>

      {error && <p role="alert">Error: {error}</p>}
    </main>
  );
}
