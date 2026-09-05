//! Compact human-readable presentation without changing structured command data.
//! 简洁易读的终端展示，不改变命令的结构化数据。

use serde_json::Value;

pub(super) fn release_list(data: &Value) -> String {
    let versions = data["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|release| release["version"].as_str())
        .collect::<Vec<_>>();
    if versions.is_empty() {
        return "No available Python versions. / 没有可用的 Python 版本。".to_owned();
    }
    format!("VERSION\n{}", versions.join("\n"))
}

pub(super) fn runtime_list(data: &Value) -> String {
    let Some(runtimes) = data["runtimes"]
        .as_array()
        .filter(|items| !items.is_empty())
    else {
        return "No registered Python. / 尚未登记 Python。\npyrudder register <python-path> --alias work"
            .to_owned();
    };
    let mut rows = vec![[
        "VERSION".to_owned(),
        "ALIAS".to_owned(),
        "STATUS".to_owned(),
        "PATH".to_owned(),
    ]];
    for runtime in runtimes {
        let aliases = runtime["aliases"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "-".to_owned());
        rows.push([
            runtime["version"].as_str().unwrap_or("-").to_owned(),
            aliases,
            short_health(runtime["health"].as_str().unwrap_or("")).to_owned(),
            display_path(runtime["root"].as_str().unwrap_or("-")),
        ]);
    }
    let widths: [usize; 3] =
        std::array::from_fn(|column| rows.iter().map(|row| row[column].len()).max().unwrap_or(0));
    rows.iter()
        .map(|row| {
            format!(
                "{:<version_width$}  {:<alias_width$}  {:<status_width$}  {}",
                row[0],
                row[1],
                row[2],
                row[3],
                version_width = widths[0],
                alias_width = widths[1],
                status_width = widths[2],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn short_health(health: &str) -> &str {
    match health {
        "Ready" => "ready",
        "Unchecked" => "unchecked",
        "PendingRemoval" => "removing",
        value if value.starts_with("Broken") => "broken",
        value if value.starts_with("Unavailable") => "unavailable",
        _ => "unknown",
    }
}

fn display_path(path: &str) -> String {
    // Remove only display prefixes; registry paths and JSON remain unchanged.
    // 仅移除展示前缀；登记路径与 JSON 均保持不变。
    let readable = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(drive) = path.strip_prefix(r"\\?\").filter(|value| {
        let bytes = value.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    }) {
        drive.to_owned()
    } else {
        path.to_owned()
    };
    // Keep every registration on one line without terminal control characters.
    // 每条登记仅占一行，不向终端输出控制字符。
    readable
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{display_path, release_list, runtime_list};
    use serde_json::json;

    #[test]
    fn available_lists_versions_without_artifact_metadata() {
        let data = json!({"releases": [
            {"version":"3.14.7", "url":"https://example.invalid", "sha256":"abcd"},
            {"version":"3.13.15"}
        ]});
        assert_eq!(release_list(&data), "VERSION\n3.14.7\n3.13.15");
        assert!(release_list(&json!({"releases":[]})).contains("没有可用"));
    }

    #[test]
    fn list_is_one_row_per_runtime_without_internal_metadata() {
        let data = json!({"runtimes": [{
            "id": "cpython-3.12.4-x64@01234567890123456789012345678901",
            "version": "3.12.4",
            "aliases": ["py312"],
            "health": "Ready",
            "root": r"\\?\D:\env\Python\Python312",
            "scripts": [r"\\?\D:\env\Python\Python312\Scripts"],
            "origin": "external"
        }]});
        let before = data.clone();
        assert_eq!(
            runtime_list(&data),
            "VERSION  ALIAS  STATUS  PATH\n3.12.4   py312  ready   D:\\env\\Python\\Python312"
        );
        assert_eq!(data, before);
    }

    #[test]
    fn unhealthy_registrations_still_have_a_short_row() {
        let output = runtime_list(&json!({"runtimes": [{
            "version": "3.12.4",
            "aliases": [],
            "health": "Broken { reason: \"a lengthy diagnostic\" }",
            "root": r"C:\Python312"
        }]}));
        assert!(output.contains("broken"));
        assert!(!output.contains("lengthy"));
        assert_eq!(output.lines().count(), 2);
    }

    #[test]
    fn empty_list_offers_a_native_registration_command() {
        let output = runtime_list(&json!({"runtimes": []}));
        assert!(output.contains("尚未登记 Python"));
        assert!(output.contains("pyrudder register"));
    }

    #[test]
    fn readable_paths_preserve_drive_unc_and_other_namespaces() {
        assert_eq!(display_path(r"\\?\C:\Python"), r"C:\Python");
        assert_eq!(display_path(r"\\?\UNC\server\Python"), r"\\server\Python");
        assert_eq!(
            display_path(r"\\?\Volume{abc}\Python"),
            r"\\?\Volume{abc}\Python"
        );
        assert_eq!(display_path("C:\\Python\ninjected"), r"C:\Python\ninjected");
    }
}
