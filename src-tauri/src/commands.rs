//! أوامر IPC. هذا الملف جسر رقيق فقط: لا منطق عمل هنا، المنطق كله في mm-core.

use std::time::Duration;

use tauri::ipc::Channel;

use crate::dto::{AppInfo, DownloadEvent, IpcError};

#[tauri::command]
pub fn app_info() -> AppInfo {
    mm_core::app_info().into()
}

/// تحميل تجريبي يبثّ تقدمه عبر Channel (أسرع وأكثر ترتيباً من Events).
#[tauri::command]
pub async fn start_demo_download(on_event: Channel<DownloadEvent>) -> Result<(), IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        mm_core::demo::run(5_000_000, 20, Duration::from_millis(100), |progress| {
            // إغلاق الواجهة أثناء التحميل لا يجب أن يوقف العملية.
            let _ = on_event.send(progress.into());
        });
    })
    .await
    .map_err(IpcError::internal)
}
