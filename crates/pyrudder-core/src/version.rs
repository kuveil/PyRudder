//! Numeric interpreter versions, not the complete package-version grammar.
//! 数字化解释器版本，不实现完整的包版本语法。
//!
//! Accepts M.m.p with optional aN/bN/rcN; rejects epochs, dev/post/local versions,
//! whitespace, leading zeroes, and implicit prerelease serials.
//! 接受 M.m.p 和可选的 aN/bN/rcN；拒绝 epoch、dev/post/local 版本、
//! 空白、前导零及省略序号的预发布格式。

use std::{fmt, str::FromStr};

use crate::{Result, error::invalid_input};

/// Maximum version input length in bytes. / 版本输入的最大字节数。
pub const MAX_VERSION_LENGTH: usize = 64;

/// Release phases in chronological order, with numeric prerelease serials.
/// 按时间顺序排列的发布阶段，预发布序号按数字比较。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReleaseLevel {
    /// Alpha release. / Alpha 预发布。
    Alpha(u32),
    /// Beta release. / Beta 预发布。
    Beta(u32),
    /// Release candidate. / 候选发布。
    Candidate(u32),
    /// Stable final release. / 稳定正式发布。
    Final,
}

/// An exact interpreter version with numeric ordering and canonical display.
/// 使用数字排序和规范显示形式的精确解释器版本。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PythonVersion {
    major: u32,
    minor: u32,
    patch: u32,
    release: ReleaseLevel,
}

impl PythonVersion {
    /// Creates a stable version; this does not assert that it exists upstream.
    /// 创建稳定版本；不代表上游实际存在此版本。
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            release: ReleaseLevel::Final,
        }
    }

    /// Sets the release phase for metadata and comparison.
    /// 设置元数据与比较所需的发布阶段。
    #[must_use]
    pub const fn with_release(mut self, release: ReleaseLevel) -> Self {
        self.release = release;
        self
    }

    /// Returns the major version. / 返回主版本号。
    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the minor version. / 返回次版本号。
    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Returns the patch version. / 返回补丁版本号。
    #[must_use]
    pub const fn patch(self) -> u32 {
        self.patch
    }

    /// Returns the release phase. / 返回发布阶段。
    #[must_use]
    pub const fn release(self) -> ReleaseLevel {
        self.release
    }

    /// Whether this is a final release. / 是否为正式发布。
    #[must_use]
    pub const fn is_stable(self) -> bool {
        matches!(self.release, ReleaseLevel::Final)
    }
}

impl FromStr for PythonVersion {
    type Err = crate::Error;

    fn from_str(input: &str) -> Result<Self> {
        if input.len() > MAX_VERSION_LENGTH || !input.is_ascii() {
            return Err(invalid_input(
                "Python version must be ASCII and at most 64 bytes",
            ));
        }
        let mut components = input.split('.');
        let major = parse_component(components.next().unwrap_or_default())?;
        let minor = parse_component(components.next().unwrap_or_default())?;
        let tail = components
            .next()
            .ok_or_else(|| invalid_input("Expected an exact version: major.minor.patch"))?;
        if components.next().is_some() {
            return Err(invalid_input("Expected exactly three version components"));
        }

        let patch_end = tail.bytes().take_while(u8::is_ascii_digit).count();
        let patch = parse_component(&tail[..patch_end])?;
        let suffix = tail[patch_end..].to_ascii_lowercase();
        let release = if suffix.is_empty() {
            ReleaseLevel::Final
        } else if let Some(serial) = suffix.strip_prefix("rc") {
            ReleaseLevel::Candidate(parse_component(serial)?)
        } else if let Some(serial) = suffix.strip_prefix('a') {
            ReleaseLevel::Alpha(parse_component(serial)?)
        } else if let Some(serial) = suffix.strip_prefix('b') {
            ReleaseLevel::Beta(parse_component(serial)?)
        } else {
            return Err(invalid_input(
                "Only aN, bN, and rcN version suffixes are supported",
            ));
        };
        Ok(Self::new(major, minor, patch).with_release(release))
    }
}

impl fmt::Display for PythonVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match self.release {
            ReleaseLevel::Alpha(serial) => write!(formatter, "a{serial}"),
            ReleaseLevel::Beta(serial) => write!(formatter, "b{serial}"),
            ReleaseLevel::Candidate(serial) => write!(formatter, "rc{serial}"),
            ReleaseLevel::Final => Ok(()),
        }
    }
}

pub(crate) fn parse_component(input: &str) -> Result<u32> {
    if input.is_empty()
        || input.len() > 10
        || !input.bytes().all(|byte| byte.is_ascii_digit())
        || (input.len() > 1 && input.starts_with('0'))
    {
        return Err(invalid_input(
            "Version numbers require decimal digits without leading zeroes",
        ));
    }
    input
        .parse()
        .map_err(|_| invalid_input("Version component exceeds the u32 limit"))
}
