//! Fixed system script hosts and type-specific argument encoding; never evaluate caller code.
//! 固定系统脚本宿主与类型专属参数编码；绝不将调用方参数作为代码求值。

#[cfg(test)]
mod tests;

use super::{environment, invalid_target, open_target, recheck_file, reject_router};
use crate::{
    file_identity::{FileIdentity, file_identity},
    native_process, pe,
    state::{WindowsStateFileSystem, check_type, io_error},
};
use pyrudder_core::{Error, ErrorKind, Result, commands::CommandKind, state::StateFileSystem};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, Metadata},
    io::{Seek, SeekFrom},
    os::windows::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

const CMD_MAX_UNITS: usize = 8_191;

pub(super) struct ScriptHost {
    kind: CommandKind,
    path: PathBuf,
    parent: PathBuf,
    file: File,
    identity: FileIdentity,
    metadata: Metadata,
}

impl ScriptHost {
    pub(super) fn prepare(kind: CommandKind) -> Result<Self> {
        let relative = match kind {
            CommandKind::Cmd | CommandKind::Bat => Path::new("cmd.exe"),
            CommandKind::PowerShell => Path::new(r"WindowsPowerShell\v1.0\powershell.exe"),
            _ => return Err(invalid_target()),
        };
        // The OS API is the authority, not caller-controlled PATH/COMSPEC/SystemRoot values.
        // 路径以操作系统 API 为准，而不是调用方可控制的 PATH/COMSPEC/SystemRoot 值。
        let path = environment::dos_path(&system_directory()?.join(relative));
        let parent = WindowsStateFileSystem
            .canonical_directory(path.parent().ok_or_else(invalid_target)?)?;
        let file = open_target(&path).map_err(|error| {
            if error.kind() == ErrorKind::CommandMissing {
                Error::new(
                    ErrorKind::BrokenRuntime,
                    "Required system script interpreter is unavailable",
                )
            } else {
                error
            }
        })?;
        let identity = file_identity(&file)
            .map_err(|error| io_error("identify script interpreter", &path, &error))?;
        reject_router(identity)?;
        let metadata = file
            .metadata()
            .map_err(|error| io_error("inspect script interpreter", &path, &error))?;
        let mut host = Self {
            kind,
            path,
            parent,
            file,
            identity,
            metadata,
        };
        host.recheck()?;
        Ok(host)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn command_line(&self, script: &Path, arguments: &[OsString]) -> Result<Vec<u16>> {
        let script = environment::dos_path(script);
        match self.kind {
            CommandKind::Cmd | CommandKind::Bat => cmd_command(&self.path, &script, arguments),
            CommandKind::PowerShell => powershell_command(&self.path, &script, arguments),
            _ => Err(invalid_target()),
        }
    }

    pub(super) fn recheck(&mut self) -> Result<()> {
        recheck_file(&self.path, &self.parent, self.identity)?;
        let metadata = self
            .file
            .metadata()
            .map_err(|error| io_error("recheck script interpreter", &self.path, &error))?;
        check_type(&metadata, false)?;
        let modified = metadata
            .modified()
            .map_err(|error| io_error("read interpreter timestamp", &self.path, &error))?;
        let original_modified = self
            .metadata
            .modified()
            .map_err(|error| io_error("read original interpreter timestamp", &self.path, &error))?;
        if metadata.len() != self.metadata.len()
            || metadata.file_attributes() != self.metadata.file_attributes()
            || modified != original_modified
        {
            return Err(Error::new(
                ErrorKind::BrokenRuntime,
                "System script interpreter changed after preparation",
            ));
        }
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| io_error("seek interpreter header", &self.path, &error))?;
        if pe::classify(&mut self.file, metadata.len())
            .map_err(|error| io_error("read interpreter header", &self.path, &error))?
            != Ok(CommandKind::PeConsole)
        {
            return Err(Error::new(
                ErrorKind::BrokenRuntime,
                "System script interpreter must be a native x64 console PE",
            ));
        }
        Ok(())
    }
}

#[allow(unsafe_code)]
fn system_directory() -> Result<PathBuf> {
    const CAPACITY: u32 = 32_768;
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: the writable UTF-16 buffer contains exactly CAPACITY elements. The reported
    // length is checked before slicing; failure never exposes uninitialized or truncated data.
    // 安全性：可写 UTF-16 缓冲区恰有 CAPACITY 个元素；切片前检查返回长度，
    // 失败时不暴露未初始化或截断的数据。
    let count = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), CAPACITY) };
    if count == 0 || count >= CAPACITY {
        return Err(Error::new(
            ErrorKind::Internal,
            "Cannot locate the Windows system directory",
        ));
    }
    buffer.truncate(usize::try_from(count).map_err(|_| invalid_target())?);
    WindowsStateFileSystem.canonical_directory(&PathBuf::from(OsString::from_wide(&buffer)))
}

fn cmd_command(image: &Path, script: &Path, arguments: &[OsString]) -> Result<Vec<u16>> {
    let mut output = Vec::new();
    cmd_literal(&mut output, image.as_os_str())?;
    // /s strips only the outer pair; each inner literal retains its own quotes. No CALL pass,
    // variable indirection, or CRT backslash escaping is used for the CMD command body.
    // /s 只剥除最外层引号，每个内部字面量保留自身引号。命令体不使用 CALL 二次解析、
    // 变量间接传参或 CRT 反斜杠转义。
    cmd_append(&mut output, " /d /s /v:off /e:on /c \"".encode_utf16())?;
    cmd_literal(&mut output, script.as_os_str())?;
    for argument in arguments {
        cmd_append(&mut output, [u16::from(b' ')])?;
        cmd_literal(&mut output, argument)?;
    }
    cmd_append(&mut output, [u16::from(b'"')])?;
    output.push(0);
    Ok(output)
}

fn cmd_literal(output: &mut Vec<u16>, value: &OsStr) -> Result<()> {
    let text = script_text(value)?;
    // Reject expansion/quote syntax rather than guessing an escape that a second parse may undo.
    // Quoted &, |, <, >, ^ and parentheses remain data at this boundary; trusted scripts may reparse.
    // 拒绝展开/引号语法，不猜测可能被二次解析撤销的转义。
    // 带引号的 &、|、<、>、^ 和括号在此边界内仍为数据；可信脚本自身可能再次解析。
    if text
        .chars()
        .any(|character| character.is_control() || matches!(character, '"' | '%' | '!'))
    {
        return Err(Error::new(
            ErrorKind::Usage,
            "CMD/BAT paths and arguments cannot contain quotes, %, !, or control characters",
        ));
    }
    cmd_append(output, [u16::from(b'"')])?;
    cmd_append(output, text.encode_utf16())?;
    cmd_append(output, [u16::from(b'"')])
}

fn cmd_append(output: &mut Vec<u16>, units: impl IntoIterator<Item = u16>) -> Result<()> {
    for unit in units {
        if output.len() >= CMD_MAX_UNITS {
            return Err(Error::new(
                ErrorKind::Usage,
                "Encoded CMD/BAT command line exceeds 8191 UTF-16 units",
            ));
        }
        output.push(unit);
    }
    Ok(())
}

fn powershell_command(image: &Path, script: &Path, arguments: &[OsString]) -> Result<Vec<u16>> {
    script_text(script.as_os_str())?;
    for argument in arguments {
        script_text(argument)?;
    }
    // -File is the last host option; later tokens are script parameters, never inline source.
    // Preserve inherited execution policy and stdin; no Bypass, profiles, or temporary script.
    // -File 是最后一个宿主选项；后续项是脚本参数，绝不作为内联源码。
    // 保留继承的执行策略与 stdin；不使用 Bypass、配置文件或临时脚本。
    let options = [
        OsStr::new("-NoLogo"),
        OsStr::new("-NoProfile"),
        OsStr::new("-File"),
        script.as_os_str(),
    ];
    native_process::command_line_iter(
        image,
        options
            .into_iter()
            .chain(arguments.iter().map(OsString::as_os_str)),
    )
}

fn script_text(value: &OsStr) -> Result<&str> {
    value
        .to_str()
        .filter(|text| !text.contains('\0'))
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Usage,
                "Script paths and arguments must be Unicode without NUL",
            )
        })
}
