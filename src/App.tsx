import { useEffect, useState } from 'react';

import type { AppInfo } from './bindings/AppInfo';
import { getAppInfo, toIpcError } from './lib/ipc';

// شاشة تشخيص مؤقتة: تثبت أن IPC والأنواع المولَّدة تعمل. تُستبدل في M2.
// لا نصوص ثابتة (§5.9): الشاشة تُزال مع أول شاشة حقيقية في Task 5.
export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAppInfo()
      .then(setInfo)
      .catch((e) => setError(toIpcError(e).message));
  }, []);

  return (
    <main style={{ fontFamily: 'system-ui, sans-serif', padding: 24, maxWidth: 480 }}>
      <h1>{info?.name ?? 'ModManager'}</h1>
      {info && (
        <p>
          {info.version} · {info.os}/{info.arch}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </main>
  );
}