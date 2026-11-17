//! Which drive a pasted link is from.

use super::Provider;

/// The provider of a share link, or `None` if it is not one Detour knows.
pub fn provider_of(url: &str) -> Option<Provider> {
    let url = url.trim();
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    let host = host.rsplit_once('@').map_or(host.as_str(), |(_, h)| h).to_string();
    let is = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if is("1drv.ms") || is("onedrive.live.com") || is("onedrive.com") || is("sharepoint.com") {
        Some(Provider::OneDrive)
    } else if is("drive.google.com") {
        Some(Provider::GoogleDrive)
    } else if is("dropbox.com") || is("db.tt") {
        Some(Provider::Dropbox)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_their_drive() {
        assert_eq!(provider_of("https://1drv.ms/f/s!AbC"), Some(Provider::OneDrive));
        assert_eq!(provider_of("https://onedrive.live.com/?id=X&cid=Y"), Some(Provider::OneDrive));
        assert_eq!(provider_of("https://contoso-my.sharepoint.com/:f:/g/personal/x"), Some(Provider::OneDrive));
        assert_eq!(provider_of("https://drive.google.com/drive/folders/1abc"), Some(Provider::GoogleDrive));
        assert_eq!(provider_of("https://www.dropbox.com/scl/fo/abc/xyz?rlkey=1"), Some(Provider::Dropbox));
        assert_eq!(provider_of("https://github.com/me/vault"), None);
        assert_eq!(provider_of("https://evil.com/onedrive.live.com"), None);
        assert_eq!(provider_of("https://notdropbox.com/x"), None);
    }
}
