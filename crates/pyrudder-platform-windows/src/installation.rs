//! Cooperative process exclusion while an installer updates this installation.
//! 安装器更新期间的协作式进程互斥。

use crate::{registry::Location, state::WindowsStateFileSystem, storage::FileLease};
use pyrudder_core::{Error, ErrorKind, Result, state::StateFileSystem};

/// Keeps a normal CLI or Python invocation out of a pending installer transaction.
/// 阻止普通 CLI 或 Python 调用进入待完成的安装事务。
///
/// # Errors
/// Reports active upgrades, malformed journals, or unavailable sharing leases.
/// 报告进行中的升级、损坏日志或不可用的共享租约。
pub fn shared_access(location: &Location) -> Result<Option<FileLease>> {
    let lock = location.config_dir.join("installer.lock");
    let lease = if lock.try_exists().map_err(|_| busy())? {
        Some(FileLease::acquire(&lock, false, false)?)
    } else {
        None
    };
    let root = location.install_dir.parent().ok_or_else(busy)?;
    let backup = root.join(".pyrudder-upgrade-backup");
    if backup.try_exists().map_err(|_| busy())? {
        let bytes = WindowsStateFileSystem
            .read_file(&backup.join("journal.json"), 32 * 1024 * 1024)?
            .ok_or_else(busy)?;
        let journal: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| busy())?;
        if journal["schema_version"] != 1
            || !matches!(journal["phase"].as_str(), Some("committed" | "rolled_back"))
        {
            return Err(busy());
        }
    }
    Ok(lease)
}

fn busy() -> Error {
    Error::new(
        ErrorKind::Busy,
        "Installer update is in progress or needs recovery; close PyRudder/Python and rerun Setup. / 安装更新正在进行或需要恢复，请关闭 PyRudder/Python 后重新运行安装器。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FileLease;
    use std::fs;

    #[test]
    fn excludes_installer_and_blocks_pending_recovery() -> Result<()> {
        let temp = tempfile::tempdir().map_err(|_| busy())?;
        let root = temp.path();
        let location = Location {
            schema_version: 1,
            config_dir: root.join("config"),
            install_dir: root.join("bin"),
            shims_dir: root.join("shims"),
        };
        let lock = location.config_dir.join("installer.lock");
        let exclusive = FileLease::acquire(&lock, true, true)?;
        assert!(shared_access(&location).is_err());
        drop(exclusive);
        let shared = shared_access(&location)?;
        assert!(FileLease::acquire(&lock, true, false).is_err());
        drop(shared);
        fs::create_dir(root.join(".pyrudder-upgrade-backup")).map_err(|_| busy())?;
        assert!(shared_access(&location).is_err());
        let path = root.join(".pyrudder-upgrade-backup").join("journal.json");
        for phase in ["preparing", "prepared", "restoring"] {
            WindowsStateFileSystem.write_atomic(
                &path,
                format!(r#"{{"schema_version":1,"phase":"{phase}"}}"#).as_bytes(),
            )?;
            assert!(shared_access(&location).is_err());
        }
        for phase in ["committed", "rolled_back"] {
            WindowsStateFileSystem.write_atomic(
                &path,
                format!(r#"{{"schema_version":1,"phase":"{phase}"}}"#).as_bytes(),
            )?;
            assert!(shared_access(&location).is_ok());
        }
        Ok(())
    }
}
