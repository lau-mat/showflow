use serde::Serialize;

#[derive(Serialize)]
pub struct PlatformInfo {
    /// "desktop" or "mobile"; the webapp never reaches this command.
    pub kind: &'static str,
    /// Value of `std::env::consts::OS`: linux, windows, macos, android, ios.
    pub os: &'static str,
}

pub fn current() -> PlatformInfo {
    PlatformInfo {
        kind: if cfg!(mobile) { "mobile" } else { "desktop" },
        os: std::env::consts::OS,
    }
}

#[tauri::command]
pub fn platform_info() -> PlatformInfo {
    current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_compile_time_platform() {
        let info = current();
        assert_eq!(info.os, std::env::consts::OS);
        assert!(info.kind == "desktop" || info.kind == "mobile");
    }
}
