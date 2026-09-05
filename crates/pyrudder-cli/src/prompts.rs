//! Interactive runtime selection and installation-directory prompts.
//! 交互式运行时选择和安装目录输入。

use std::io::{self, BufRead, IsTerminal};
use std::path::{Path, PathBuf};

use console::Term;
use dialoguer::{Select, theme::SimpleTheme};
use pyrudder_core::{Error, ErrorKind, Result};

/// Chooses a listed runtime; Escape or q cancels without selecting one.
/// 选择列表中的运行时；Escape 或 q 取消选择。
pub(crate) fn choose_version(items: &[String]) -> Result<Option<usize>> {
    let term = interactive_terminal()?;
    if items.is_empty() {
        return Ok(None);
    }
    term.write_line(
        "Up/Down: choose | Enter: install | Esc/q: cancel / 上下选择，回车安装，Esc/q 取消",
    )
    .map_err(|error| prompt_io_error(&error))?;
    let selection = Select::with_theme(&SimpleTheme)
        .with_prompt("Python version / Python 版本")
        .items(items)
        .default(0)
        .max_length(10)
        .interact_on_opt(&term);
    // Restore the cursor even when the terminal closes during selection.
    // 即使终端在选择过程中关闭，也尝试恢复光标。
    let _ = term.show_cursor();
    match selection {
        Ok(selected) => Ok(selected),
        Err(error) => {
            let error = io::Error::from(error);
            if matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof
            ) {
                Ok(None)
            } else {
                Err(prompt_io_error(&error))
            }
        }
    }
}

/// Reads and validates a destination; a confirmed empty line uses the caller's default.
/// 读取并校验目标目录；确认输入空行时使用调用方的默认目录。
pub(crate) fn installation_directory(
    default: &Path,
    validate: impl Fn(&str) -> Result<PathBuf>,
) -> Result<Option<PathBuf>> {
    let term = interactive_terminal()?;
    term.write_line(&format!(
        "Default directory / 默认目录: {}",
        default.display()
    ))
    .map_err(|error| prompt_io_error(&error))?;
    term.write_line("Enter: use default | Ctrl+C: cancel / 直接回车使用默认目录，Ctrl+C 取消")
        .map_err(|error| prompt_io_error(&error))?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    loop {
        term.write_str("Installation directory / 安装目录: ")
            .and_then(|()| term.flush())
            .map_err(|error| prompt_io_error(&error))?;
        let Some(value) = read_directory_input(&mut input)? else {
            return Ok(None);
        };
        match validate(&value) {
            Ok(path) => return Ok(Some(path)),
            Err(error) => {
                term.write_line(&format!("{error}"))
                    .map_err(|error| prompt_io_error(&error))?;
            }
        }
    }
}

fn interactive_terminal() -> Result<Term> {
    let term = Term::stderr();
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() || !term.is_term() {
        return Err(Error::new(
            ErrorKind::Usage,
            "Interactive installation requires a terminal / 交互安装需要终端",
        ));
    }
    Ok(term)
}

fn read_directory_input(input: &mut impl BufRead) -> Result<Option<String>> {
    let mut value = String::new();
    match input.read_line(&mut value) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(None),
        Err(error) => return Err(prompt_io_error(&error)),
    }
    // EOF must not be mistaken for Enter and start a default installation.
    // 不能把 EOF 当成回车，否则可能意外触发默认安装。
    if !value.ends_with('\n') || value.contains(['\u{3}', '\u{4}', '\u{1a}']) {
        return Ok(None);
    }
    Ok(Some(value.trim().to_owned()))
}

fn prompt_io_error(error: &io::Error) -> Error {
    Error::new(
        ErrorKind::Usage,
        format!("Cannot read installation prompt / 无法读取安装输入: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::read_directory_input;
    use std::io::Cursor;

    #[test]
    fn eof_does_not_accept_the_default_directory() -> pyrudder_core::Result<()> {
        assert_eq!(read_directory_input(&mut Cursor::new(b""))?, None);
        assert_eq!(
            read_directory_input(&mut Cursor::new(b"D:\\unfinished"))?,
            None
        );
        Ok(())
    }

    #[test]
    fn enter_accepts_the_default_directory() -> pyrudder_core::Result<()> {
        for line in ["\n", "\r\n", "   \r\n"] {
            assert_eq!(
                read_directory_input(&mut Cursor::new(line))?,
                Some(String::new())
            );
        }
        Ok(())
    }

    #[test]
    fn control_characters_cancel_instead_of_selecting_a_directory() -> pyrudder_core::Result<()> {
        for line in ["\u{3}\n", "\u{4}\n", "\u{1a}\r\n"] {
            assert_eq!(read_directory_input(&mut Cursor::new(line))?, None);
        }
        Ok(())
    }

    #[test]
    fn unicode_and_spaces_in_confirmed_paths_are_preserved() -> pyrudder_core::Result<()> {
        assert_eq!(
            read_directory_input(&mut Cursor::new(" D:\\开发目录\\Python 3.14\r\n"))?,
            Some("D:\\开发目录\\Python 3.14".to_owned())
        );
        Ok(())
    }
}
