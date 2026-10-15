//! أنواع تعبر حدّ IPC. كل نوع هنا يُولَّد له ملف TypeScript في src/bindings
//! عند تشغيل `cargo test --workspace`.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
}

impl From<mm_core::AppInfo> for AppInfo {
    fn from(info: mm_core::AppInfo) -> Self {
        Self {
            name: info.name.to_owned(),
            version: info.version.to_owned(),
            os: info.os.to_owned(),
            arch: info.arch.to_owned(),
        }
    }
}

/// أحداث التقدم المرسلة عبر Channel. تصل إلى الواجهة بالشكل
/// `{ event: 'started', data: { total } }` و`{ event: 'finished' }`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
#[ts(export)]
pub enum DownloadEvent {
    Started {
        #[ts(type = "number")]
        total: u64,
    },
    Advanced {
        #[ts(type = "number")]
        downloaded: u64,
    },
    Finished,
}

impl From<mm_core::demo::Progress> for DownloadEvent {
    fn from(progress: mm_core::demo::Progress) -> Self {
        use mm_core::demo::Progress;
        match progress {
            Progress::Started { total } => Self::Started { total },
            Progress::Advanced { downloaded } => Self::Advanced { downloaded },
            Progress::Finished => Self::Finished,
        }
    }
}

/// رمز الخطأ المرسل للواجهة؛ تترجمه الواجهة إلى نص حسب لغة المستخدم.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    Internal,
}

/// خطأ مُنمَّط يعبر حدّ IPC بدل سلسلة نصية خام.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

impl IpcError {
    pub fn internal(message: impl ToString) -> Self {
        Self {
            code: ErrorCode::Internal,
            message: message.to_string(),
        }
    }
}
