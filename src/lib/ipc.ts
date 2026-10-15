import { Channel, invoke } from '@tauri-apps/api/core';

import type { AppInfo } from '../bindings/AppInfo';
import type { DownloadEvent } from '../bindings/DownloadEvent';
import type { IpcError } from '../bindings/IpcError';

// الواجهة تصل إلى النواة عبر هذا الملف فقط.

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info');
}

export async function startDemoDownload(
  onEvent: (event: DownloadEvent) => void,
): Promise<void> {
  const channel = new Channel<DownloadEvent>();
  channel.onmessage = onEvent;
  // اسم المعامل في Rust هو on_event، وTauri يحوّله إلى onEvent.
  await invoke('start_demo_download', { onEvent: channel });
}

/** يحوّل أي خطأ يرفضه invoke إلى IpcError (Tauri يرفض بالقيمة المُسلسَلة نفسها). */
export function toIpcError(e: unknown): IpcError {
  if (typeof e === 'object' && e !== null && 'code' in e && 'message' in e) {
    return e as IpcError;
  }
  return { code: 'internal', message: String(e) };
}
