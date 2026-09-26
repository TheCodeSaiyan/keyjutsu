//! Where to get what KeyJutsu relies on but does not install itself.
//!
//! Each is the maker's own page. The desktop app opens one only when it is
//! on this list: the window asks for a link, and Rust decides whether it is
//! one KeyJutsu offers, so a compromised page cannot use KeyJutsu to open an
//! address of its choosing.

/// PowerShell 7, from Microsoft.
pub const POWERSHELL: &str =
    "https://learn.microsoft.com/powershell/scripting/install/install-powershell-on-windows";
/// Git for Windows.
pub const GIT: &str = "https://git-scm.com/downloads/win";
/// The WebView2 runtime the desktop app draws its window with, from Microsoft.
pub const WEBVIEW2: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// Every link KeyJutsu offers.
pub fn known() -> Vec<&'static str> {
    let mut all = vec![POWERSHELL, GIT, WEBVIEW2];
    all.extend(keyjutsu_agent::AgentKind::ALL.iter().map(|k| k.install_url()));
    all
}

/// Whether `url` is one KeyJutsu offers, exactly.
pub fn is_known(url: &str) -> bool {
    known().contains(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_links_keyjutsu_offers_are_known() {
        assert!(is_known(POWERSHELL));
        assert!(is_known(keyjutsu_agent::AgentKind::Codex.install_url()));
        for other in [
            "https://example.com/",
            "file:///C:/Windows/System32/calc.exe",
            &format!("{GIT}?x=1"),
            &format!(" {GIT}"),
            "",
        ] {
            assert!(!is_known(other), "{other}");
        }
        for url in known() {
            assert!(url.starts_with("https://"), "{url}");
        }
    }
}
