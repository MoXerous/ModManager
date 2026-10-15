//! نواة ModManager.
//!
//! قاعدة ملزِمة: هذه المكتبة لا تعرف شيئاً عن Tauri أو الواجهة.
//! تطبيق Tauri وأداة CLI عميلان لها.

pub mod demo;

/// اسم التطبيق كما يظهر للمستخدم.
pub const APP_NAME: &str = "ModManager";

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
