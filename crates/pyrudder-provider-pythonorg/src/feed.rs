//! Signed, bounded, paginated official feed and stable x64 selection.
//! 有签名、有界、支持分页的官方索引和稳定版 x64 选择。

use super::{
    DEFAULT_INDEX_URL, PythonOrgProvider,
    transfer::{client, get_bytes, trusted_url},
};
use pyrudder_core::{
    Error, ErrorKind, Result, selector::VersionSelector, state::StateFileSystem,
    version::PythonVersion,
};
use pyrudder_platform_windows::{
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease},
    trust::verify_python_index,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
};

/// A stable standard `CPython` x64 artifact from a verified feed.
/// 来自已验证索引的标准稳定版 `CPython` x64 产物。
#[derive(Clone, Debug, Serialize)]
pub struct Release {
    /// Exact stable version. / 精确稳定版本。
    pub version: String,
    /// Allowlisted HTTPS artifact URL. / 允许列表内的 HTTPS 产物地址。
    pub url: String,
    /// Signed SHA-256 expectation. / 签名索引中的 SHA-256 预期值。
    pub sha256: String,
}

#[derive(Deserialize)]
struct Feed {
    versions: Vec<Entry>,
    next: Option<String>,
}
#[derive(Deserialize)]
struct Entry {
    schema: u32,
    id: String,
    company: String,
    tag: String,
    #[serde(rename = "sort-version")]
    version: String,
    executable: String,
    url: String,
    // Legacy NuGet entries omit hashes; supported ZIPs still require SHA-256 below.
    // 旧版 NuGet 条目省略摘要；受支持的 ZIP 仍必须通过下方 SHA-256 校验。
    #[serde(default)]
    hash: BTreeMap<String, String>,
}

impl PythonOrgProvider {
    /// Retrieve and verify all bounded feed pages, or use explicitly offline cache.
    /// 获取并验证有界索引分页，或使用显式离线缓存。
    ///
    /// # Errors
    /// Rejects networking, signature, schema, pagination and conflicting digest errors.
    /// 拒绝网络、签名、schema、分页和摘要冲突错误。
    pub fn releases(cache: &Path, offline: bool) -> Result<Vec<Release>> {
        let _directory = DirectoryLease::acquire(cache, true)?;
        let _lock = FileLease::acquire(&cache.join("pythonorg-feed.lock"), true, true)?;
        let http = client()?;
        let mut url = trusted_url(DEFAULT_INDEX_URL)?;
        let mut visited = BTreeSet::new();
        let mut releases = BTreeMap::new();
        for _ in 0..16 {
            if !visited.insert(url.to_string()) {
                return Err(integrity("Index pagination cycle"));
            }
            let key = format!("{:x}", Sha256::digest(url.as_str().as_bytes()));
            let json_path = cache.join(format!("pythonorg-{key}.json"));
            let cat_path = cache.join(format!("pythonorg-{key}.cat"));
            let bytes = if offline {
                verify_python_index(&json_path, &cat_path, true)?
            } else {
                let bytes = get_bytes(&http, &url, 8 * 1024 * 1024)?;
                let signature =
                    get_bytes(&http, &trusted_url(&format!("{url}.cat"))?, 4 * 1024 * 1024)?;
                let staged_json = staged(cache, &bytes)?;
                let staged_cat = staged(cache, &signature)?;
                let bytes = verify_python_index(&staged_json, &staged_cat, false)?;
                WindowsStateFileSystem.write_atomic(&json_path, &bytes)?;
                WindowsStateFileSystem.write_atomic(&cat_path, &signature)?;
                bytes
            };
            let feed = parse_feed(&bytes)?;
            if feed.versions.len() > 10_000 {
                return Err(integrity("Index entry limit exceeded"));
            }
            for entry in &feed.versions {
                if let Some(release) = convert(entry)? {
                    let version: PythonVersion = release.version.parse()?;
                    if let Some(previous) = releases.insert(version, release.clone()) {
                        if previous.sha256 != release.sha256 || previous.url != release.url {
                            return Err(integrity("Conflicting artifacts for one Python version"));
                        }
                    }
                }
            }
            match feed.next.filter(|next| !next.is_empty()) {
                None => return Ok(releases.into_values().rev().collect()),
                Some(next) => {
                    url = trusted_url(
                        url.join(&next)
                            .map_err(|_| integrity("Invalid next index URL"))?
                            .as_str(),
                    )?;
                    if !Path::new(url.path())
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                    {
                        return Err(integrity("Invalid next index suffix"));
                    }
                }
            }
        }
        Err(integrity("Index page limit exceeded"))
    }

    /// Select latest, a minor series, or an exact stable release.
    /// 选择最新版、次版本系列或精确稳定版本。
    ///
    /// # Errors
    /// Rejects unsupported selectors and absent artifacts.
    /// 拒绝不支持的选择器和缺失的产物。
    pub fn select(releases: &[Release], selector: &str) -> Result<Release> {
        let parsed = if selector == "latest" {
            None
        } else {
            Some(selector.parse::<VersionSelector>()?)
        };
        for release in releases {
            let version: PythonVersion = release.version.parse()?;
            let matches = match &parsed {
                None => true,
                Some(VersionSelector::Exact(exact)) => *exact == version,
                Some(VersionSelector::Minor { major, minor }) => {
                    *major == version.major() && *minor == version.minor()
                }
                Some(VersionSelector::Runtime(id)) => {
                    id.installation().is_none() && id.version() == version
                }
                _ => {
                    return Err(Error::new(
                        ErrorKind::Usage,
                        "Downloads require latest, 3.x, or an exact stable version",
                    ));
                }
            };
            if matches {
                return Ok(release.clone());
            }
        }
        Err(Error::new(
            ErrorKind::NotInstalled,
            "No matching stable CPython x64 artifact in the official feed",
        ))
    }
}

fn parse_feed(bytes: &[u8]) -> Result<Feed> {
    serde_json::from_slice(bytes).map_err(|error| {
        // Bound remote-derived diagnostics and keep them on a single terminal line.
        // 限制源自远端数据的诊断长度，并保持为单行终端输出。
        let detail: String = error
            .to_string()
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .take(240)
            .collect();
        integrity(&format!(
            "Invalid official feed JSON/schema at line {}, column {}: {detail}",
            error.line(),
            error.column()
        ))
    })
}

fn staged(directory: &Path, bytes: &[u8]) -> Result<tempfile::TempPath> {
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| integrity("Cannot stage index verification"))?;
    file.write_all(bytes)
        .map_err(|_| integrity("Cannot write staged index"))?;
    file.as_file()
        .sync_all()
        .map_err(|_| integrity("Cannot flush staged index"))?;
    Ok(file.into_temp_path())
}

fn convert(entry: &Entry) -> Result<Option<Release>> {
    let Ok(version) = entry.version.parse::<PythonVersion>() else {
        return Ok(None);
    };
    if !version.is_stable() || version.major() != 3 || entry.company != "PythonCore" {
        return Ok(None);
    }
    let tag = format!("{}.{}-64", version.major(), version.minor());
    if entry.tag != tag || entry.id != format!("pythoncore-{tag}") {
        return Ok(None);
    }
    if entry.schema != 1
        || !matches!(
            entry.executable.as_str(),
            ".\\python.exe" | "python.exe" | "./python.exe"
        )
    {
        return Err(integrity("Unsupported official runtime schema"));
    }
    // Recognize only the signed feed's exact legacy x64 NuGet layout as unsupported.
    // 仅将签名索引中精确匹配旧版 x64 NuGet 布局的条目标记为不支持。
    // This is an exclusion, not permission to download from another origin or omit hashes.
    // 这是排除规则，不代表允许从其他来源下载或省略摘要验证。
    if is_legacy_nuget(&entry.url, version) {
        return Ok(None);
    }
    let url = trusted_url(&entry.url)?;
    if !url
        .path()
        .ends_with(&format!("/python-{version}-amd64.zip"))
    {
        return Err(integrity("Expected complete standard amd64 runtime ZIP"));
    }
    let digest = entry
        .hash
        .get("sha256")
        .ok_or_else(|| integrity("Missing signed SHA-256"))?
        .to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(integrity("Invalid signed SHA-256"));
    }
    Ok(Some(Release {
        version: version.to_string(),
        url: url.to_string(),
        sha256: digest,
    }))
}

fn is_legacy_nuget(url: &str, version: PythonVersion) -> bool {
    let Some(path) = url.strip_prefix("https://api.nuget.org/v3-flatcontainer/python/") else {
        return false;
    };
    let Some((package, filename)) = path.split_once('/') else {
        return false;
    };
    if filename != format!("python.{package}.nupkg") {
        return false;
    }
    let version = version.to_string();
    // NuGet can append a package revision: Python 3.9.11 uses package 3.9.11.1.
    // Match the interpreter version and the same package in both URL components.
    // NuGet 可追加包修订号：Python 3.9.11 对应包 3.9.11.1。
    // 必须匹配解释器版本，并保证 URL 目录和文件名包含同一个包版本。
    package == version
        || package
            .strip_prefix(&format!("{version}."))
            .and_then(|revision| revision.parse::<u32>().ok().map(|value| (revision, value)))
            .is_some_and(|(revision, value)| value > 0 && revision == value.to_string())
}

pub(super) fn integrity(message: &str) -> Error {
    Error::new(ErrorKind::Integrity, message)
}

#[cfg(test)]
mod tests {
    use super::{Feed, convert, integrity, parse_feed};
    use pyrudder_core::Result;
    use serde_json::{Value, json};

    fn document(url: &str) -> Value {
        json!({
            "versions": [{
                "schema": 1,
                "id": "pythoncore-3.10-64",
                "company": "PythonCore",
                "tag": "3.10-64",
                "sort-version": "3.10.11",
                "executable": "./python.exe",
                "url": url
            }]
        })
    }

    fn parse(document: &Value) -> Result<Feed> {
        parse_feed(document.to_string().as_bytes())
    }

    #[test]
    fn known_legacy_nuget_without_hash_is_excluded() -> Result<()> {
        let feed = parse(&document(
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11/python.3.10.11.nupkg",
        ))?;
        assert!(convert(&feed.versions[0])?.is_none());
        Ok(())
    }

    #[test]
    fn signed_legacy_nuget_package_revisions_are_excluded() -> Result<()> {
        for (version, minor, package) in
            [("3.9.11", "3.9", "3.9.11.1"), ("3.5.2", "3.5", "3.5.2.2")]
        {
            let mut document = document(&format!(
                "https://api.nuget.org/v3-flatcontainer/python/{package}/python.{package}.nupkg"
            ));
            document["versions"][0]["id"] = json!(format!("pythoncore-{minor}-64"));
            document["versions"][0]["tag"] = json!(format!("{minor}-64"));
            document["versions"][0]["sort-version"] = json!(version);
            assert!(convert(&parse(&document)?.versions[0])?.is_none());
        }
        Ok(())
    }

    #[test]
    fn supported_zip_still_requires_signed_sha256() -> Result<()> {
        let mut document =
            document("https://www.python.org/ftp/python/3.10.11/python-3.10.11-amd64.zip");
        let feed = parse(&document)?;
        assert!(convert(&feed.versions[0]).is_err());
        document["versions"][0]["hash"] = json!({"sha256": "0".repeat(64)});
        let feed = parse(&document)?;
        assert!(convert(&feed.versions[0])?.is_some());
        document["versions"][0]["hash"] = json!({"sha256": "invalid"});
        let feed = parse(&document)?;
        assert!(convert(&feed.versions[0]).is_err());
        Ok(())
    }

    #[test]
    fn legacy_exclusion_does_not_accept_mismatched_urls() -> Result<()> {
        for url in [
            "https://api.nuget.org/v3-flatcontainer/python/3.10.10/python.3.10.10.nupkg",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11/python.3.10.11.nupkg?x=1",
            "https://example.invalid/v3-flatcontainer/python/3.10.11/python.3.10.11.nupkg",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.1/python.3.10.11.2.nupkg",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.1/python.3.10.11.1.nupkg?x=1",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.1/python.3.10.11.1.nupkg#x",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.01/python.3.10.11.01.nupkg",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.-1/python.3.10.11.-1.nupkg",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.1.1/python.3.10.11.1.1.nupkg",
            "https://api.nuget.org/v3-flatcontainer/python/3.10.11.1/extra/python.3.10.11.1.nupkg",
        ] {
            assert!(convert(&parse(&document(url))?.versions[0]).is_err());
        }
        Ok(())
    }

    #[test]
    fn legacy_exclusion_still_requires_supported_identity_and_schema() -> Result<()> {
        let mut document =
            document("https://api.nuget.org/v3-flatcontainer/python/3.10.11/python.3.10.11.nupkg");
        document["versions"][0]["schema"] = json!(2);
        assert!(convert(&parse(&document)?.versions[0]).is_err());
        document["versions"][0]["schema"] = json!(1);
        document["versions"][0]["executable"] = json!("other.exe");
        assert!(convert(&parse(&document)?.versions[0]).is_err());
        document["versions"][0]["hash"] = Value::Null;
        assert!(parse(&document).is_err());
        Ok(())
    }

    #[test]
    fn schema_diagnostics_include_bounded_location_and_reason() -> Result<()> {
        let mut document = document("https://www.python.org/ftp/python/example.zip");
        document["versions"][0]["schema"] = json!("x".repeat(4096));
        let error = parse(&document)
            .err()
            .ok_or_else(|| integrity("Wrong field type must fail"))?;
        assert!(error.message().contains("line 1, column"));
        assert!(error.message().contains("invalid type"));
        assert!(error.message().len() < 350);
        assert!(!error.message().chars().any(char::is_control));
        Ok(())
    }
}
