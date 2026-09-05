//! Terminal-only download feedback; machine-readable output remains untouched.
//! 仅面向终端的下载反馈，不影响机器可读输出。

use super::{App, usage};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use pyrudder_core::Result;
use pyrudder_provider_pythonorg::{DownloadPhase, DownloadProgress, PythonOrgProvider, Release};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::Duration,
};

impl App {
    pub(super) fn download_archive(&self, release: &Release, offline: bool) -> Result<PathBuf> {
        let display = if self.progress && io::stderr().is_terminal() {
            Some(DownloadDisplay::new(&release.version)?)
        } else {
            None
        };
        let result = PythonOrgProvider::download_with_progress(
            release,
            &self.configuration.paths.downloads_dir,
            offline,
            |event| {
                if let Some(display) = &display {
                    display.update(event);
                }
            },
        );
        if let Some(display) = display {
            if result.is_err() {
                display
                    .bar
                    .abandon_with_message("下载未完成 / Download failed");
            }
        }
        result
    }
}

struct DownloadDisplay {
    bar: ProgressBar,
    bounded: ProgressStyle,
    unbounded: ProgressStyle,
    status: ProgressStyle,
}

impl DownloadDisplay {
    fn new(version: &str) -> Result<Self> {
        let bounded = style(
            "{prefix} {spinner} {msg}\n[{wide_bar}] {whole_percent:>3}% {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}",
        )?;
        let unbounded = style("{prefix} {spinner} {msg}\n{bytes} {bytes_per_sec}")?;
        let status = style("{prefix} {spinner} {msg}\n{bytes}")?;
        // Native Windows consoles often omit TERM. The caller already verified stderr is a TTY.
        // 原生 Windows 终端通常没有 TERM；调用方已验证 stderr 确实是终端。
        let target =
            ProgressDrawTarget::term_like_with_hz(Box::new(console::Term::buffered_stderr()), 10);
        let bar = ProgressBar::hidden();
        bar.set_prefix(format!("Python {version}"));
        bar.set_style(status.clone());
        bar.set_draw_target(target);
        bar.enable_steady_tick(Duration::from_millis(100));
        Ok(Self {
            bar,
            bounded,
            unbounded,
            status,
        })
    }

    fn update(&self, event: DownloadProgress) {
        match event.phase {
            DownloadPhase::Connecting => {
                self.bar.set_style(self.status.clone());
                self.bar.disable_steady_tick();
                reset_transfer_baseline(&self.bar, event.downloaded);
                self.bar.enable_steady_tick(Duration::from_millis(100));
                self.bar
                    .set_message(format!("连接中 / Connecting ({})", event.attempt));
            }
            DownloadPhase::Downloading => {
                if let Some(total) = event.total {
                    self.bar.set_length(total);
                    self.bar.set_style(self.bounded.clone());
                } else {
                    self.bar.set_style(self.unbounded.clone());
                }
                self.bar.set_position(event.downloaded);
                self.bar.set_message("下载中 / Downloading");
            }
            DownloadPhase::Verifying => {
                self.bar.set_style(self.status.clone());
                self.bar.set_position(event.downloaded);
                self.bar.set_message("校验 SHA-256 / Verifying SHA-256");
            }
            DownloadPhase::Complete | DownloadPhase::Cached => {
                self.bar.set_position(event.downloaded);
                self.bar
                    .finish_with_message(if event.phase == DownloadPhase::Cached {
                        "缓存已校验 / Verified cache"
                    } else {
                        "下载并校验完成 / Download verified"
                    });
            }
        }
    }
}

fn style(template: &str) -> Result<ProgressStyle> {
    ProgressStyle::with_template(template)
        .map(|value| {
            value.progress_chars("#>-").with_key(
                "whole_percent",
                |state: &ProgressState, output: &mut dyn std::fmt::Write| {
                    let _ = write!(output, "{}", whole_percent(state.pos(), state.len()));
                },
            )
        })
        .map_err(|_| usage("Invalid download progress template / 下载进度模板无效"))
}

fn whole_percent(position: u64, total: Option<u64>) -> u64 {
    total
        .filter(|total| *total > 0)
        .map_or(0, |total| (position.saturating_mul(100) / total).min(100))
}

fn reset_transfer_baseline(bar: &ProgressBar, offset: u64) {
    // Existing bytes contribute to completion, never to the current network transfer rate.
    // 已有字节参与完成比例，但绝不能计入当前网络传输速度。
    bar.set_position(offset);
    bar.tick();
    bar.reset_eta();
}

#[cfg(test)]
mod tests {
    use super::{reset_transfer_baseline, whole_percent};
    use indicatif::ProgressBar;

    #[test]
    fn resumed_bytes_do_not_count_as_new_download_speed() {
        let bar = ProgressBar::hidden();
        reset_transfer_baseline(&bar, 35_000_000);
        assert_eq!(bar.position(), 35_000_000);
        assert!(bar.per_sec().abs() < f64::EPSILON);
        reset_transfer_baseline(&bar, 0);
        assert_eq!(bar.position(), 0);
        assert!(bar.per_sec().abs() < f64::EPSILON);
    }

    #[test]
    fn percentage_does_not_round_incomplete_downloads_up_to_one_hundred() {
        assert_eq!(whole_percent(999, Some(1_000)), 99);
        assert_eq!(whole_percent(1_000, Some(1_000)), 100);
        assert_eq!(whole_percent(1_000, None), 0);
    }
}
