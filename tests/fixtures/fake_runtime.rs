//! Synthetic discovery fixtures: header bytes and inert scripts, never executable test programs.
//! 合成发现夹具：文件头字节和惰性脚本，不是用于执行的测试程序。

use pyrudder_core::runtime::{
    InstallationId, RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord,
};

mod pe_header;
pub use pe_header::pe_header;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct FakeRuntime {
    _temporary: tempfile::TempDir,
    pub record: RuntimeRecord,
}

impl FakeRuntime {
    pub fn new(version: &str, installation: u128) -> TestResult<Self> {
        let temporary = tempfile::Builder::new()
            .prefix("pyrudder-command-tests-")
            .tempdir()?;
        let root = temporary.path().join("Python 空间");
        fs::create_dir_all(root.join("Scripts"))?;
        let id =
            RuntimeId::new(version.parse()?)?.with_installation(InstallationId::new(installation)?);
        let mut record = RuntimeRecord::new(
            id,
            root.clone(),
            vec![root.join("Scripts")],
            RuntimeOrigin::External {
                registered_executable: root.join("python.exe"),
            },
        )?;
        record.set_health(RuntimeHealth::Ready);
        Ok(Self {
            _temporary: temporary,
            record,
        })
    }

    pub fn write(
        &self,
        relative: impl AsRef<Path>,
        bytes: impl AsRef<[u8]>,
    ) -> io::Result<PathBuf> {
        let path = self.record.root().join(relative);
        fs::write(&path, bytes)?;
        Ok(path)
    }
}
