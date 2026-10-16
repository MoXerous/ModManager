//! نواة ModManager.
//!
//! قاعدة ملزِمة: هذه المكتبة لا تعرف شيئاً عن Tauri أو الواجهة.
//! تطبيق Tauri وأداة CLI عميلان لها.

pub mod db;
pub mod demo;
pub mod downloader;
pub mod safety;

/// اسم التطبيق كما يظهر للمستخدم.
pub const APP_NAME: &str = "ModManager";

/// User-Agent لكل طلبات الشبكة الصادرة (§5.11).
/// قيمة مؤقتة للتطوير فقط — تُستبدل بقيمة الإصدار مع رابط GitHub.
/// بوابة الإصدار أدناه تُفشل بناء release ما دامت هذه القيمة مؤقتة.
pub const USER_AGENT: &str = "ModManager/0.1.0-dev (dev build)";

#[cfg(not(debug_assertions))]
compile_error!(
    "USER_AGENT is still the temporary dev value; set the release value (with GitHub URL) before building release"
);

/// معلومات الإصدار والنظام.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

pub fn app_info() -> AppInfo {
    AppInfo {
        name: APP_NAME,
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

#[cfg(test)]
pub(crate) fn test_tempdir() -> tempfile::TempDir {
    // Inside the workspace target/ dir: git-ignored, same volume as the
    // build, and never the system temp — C: stays untouched.
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-tmp");
    if std::fs::create_dir_all(&base).is_ok() {
        if let Ok(d) = tempfile::Builder::new()
            .prefix("mm-test-")
            .tempdir_in(&base)
        {
            return d;
        }
    }
    tempfile::tempdir().expect("temp dir")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_fields_are_not_empty() {
        let info = app_info();
        assert_eq!(info.name, "ModManager");
        assert!(!info.version.is_empty());
        assert!(!info.os.is_empty());
        assert!(!info.arch.is_empty());
    }
}
