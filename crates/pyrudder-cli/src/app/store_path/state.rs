//! Bounded repair journals and single-target PATH transformations.
//! 有界修复日志与只针对单一目标的 PATH 变换。

use super::super::usage;
use pyrudder_core::Result;
use serde::{Deserialize, Serialize};

pub(super) const JOURNAL_LIMIT: usize = 262_144;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ValueKind {
    String,
    ExpandString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PathValue {
    pub kind: ValueKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Pending,
    Applied,
    Restored,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub schema_version: u32,
    pub owner_sid: String,
    pub target: String,
    pub install_dir: String,
    pub shims_dir: String,
    pub before: PathValue,
    pub after: PathValue,
    pub phase: Phase,
}

impl PathValue {
    pub fn validate(&self) -> Result<()> {
        if self.text.encode_utf16().count() >= 32_766 || self.text.contains('\0') {
            return Err(usage(
                "Store PATH repair encountered invalid or oversized PATH data",
            ));
        }
        Ok(())
    }

    pub fn contains(&self, target: &str) -> bool {
        entries(&self.text)
            .iter()
            .any(|entry| same_entry(entry, target))
    }

    pub fn without(&self, target: &str) -> Self {
        Self {
            kind: self.kind,
            text: entries(&self.text)
                .into_iter()
                .filter(|entry| !same_entry(entry, target))
                .collect::<Vec<_>>()
                .join(";"),
        }
    }
}

impl Journal {
    pub fn validate(&self, owner_sid: &str, target: &str) -> Result<()> {
        self.before.validate()?;
        self.after.validate()?;
        if self.schema_version != 1
            || self.owner_sid != owner_sid
            || self.target != target
            || !self.before.contains(target)
            || self.before.without(target) != self.after
        {
            return Err(usage(
                "Store PATH journal is not a valid single-target repair",
            ));
        }
        Ok(())
    }

    pub fn restoration(&self, current: Option<&PathValue>) -> Result<Option<PathValue>> {
        self.validate(&self.owner_sid, &self.target)?;
        if let Some(value) = current {
            value.validate()?;
        }
        if self.phase == Phase::Restored
            || current.is_some_and(|value| value.contains(&self.target))
        {
            return Ok(None);
        }
        if current == Some(&self.after) {
            // Rebuild from current entries plus validated target entries, never an arbitrary snapshot.
            // 从当前条目及已验证目标条目重建，绝不直接回写任意快照。
            let before_entries = entries(&self.before.text);
            let mut remaining = if before_entries
                .iter()
                .any(|entry| !same_entry(entry, &self.target))
            {
                self.after.text.split(';').collect::<Vec<_>>()
            } else {
                Vec::new()
            }
            .into_iter();
            let mut restored = Vec::new();
            for entry in before_entries {
                if same_entry(entry, &self.target) {
                    restored.push(entry);
                } else {
                    restored.push(
                        remaining
                            .next()
                            .ok_or_else(|| usage("Store PATH journal lost its original anchors"))?,
                    );
                }
            }
            if remaining.next().is_some() {
                return Err(usage(
                    "Store PATH journal contains unexpected remaining entries",
                ));
            }
            let result = PathValue {
                kind: self.after.kind,
                text: restored.join(";"),
            };
            result.validate()?;
            return Ok(Some(result));
        }
        if self.phase == Phase::Pending {
            return Err(usage(
                "Store PATH repair was interrupted and PATH changed again; refusing ambiguous restoration",
            ));
        }
        let kind = current.map_or(self.before.kind, |value| value.kind);
        let text = current.map_or("", |value| value.text.as_str());
        let mut current_entries = entries(text);
        let old_entries = entries(&self.before.text);
        let first = old_entries
            .iter()
            .position(|entry| same_entry(entry, &self.target))
            .ok_or_else(|| usage("Store PATH journal has no target entry"))?;
        let previous = old_entries[..first]
            .iter()
            .rev()
            .find(|entry| !same_entry(entry, &self.target))
            .and_then(|anchor| unique_position(&current_entries, anchor));
        let next = old_entries[first + 1..]
            .iter()
            .find(|entry| !same_entry(entry, &self.target))
            .and_then(|anchor| unique_position(&current_entries, anchor));
        let position = match (previous, next) {
            (Some(left), Some(right)) if left < right => right,
            (None, Some(right)) => right,
            (Some(left), _) => left + 1,
            _ => current_entries.len(),
        };
        // Only add the fixed derived target; unrelated current entries keep their exact order.
        // 仅加入固定派生的目标；当前其余条目及顺序保持原样。
        current_entries.insert(position, &self.target);
        let result = PathValue {
            kind,
            text: current_entries.join(";"),
        };
        result.validate()?;
        Ok(Some(result))
    }
}

pub(super) fn entries(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(';').collect()
    }
}

pub(super) fn same_entry(entry: &str, target: &str) -> bool {
    let entry = entry
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(entry);
    entry
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(target.trim_end_matches('\\'))
}

fn unique_position(entries: &[&str], anchor: &str) -> Option<usize> {
    let mut positions = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| **entry == anchor)
        .map(|(index, _)| index);
    let first = positions.next()?;
    positions.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::{Journal, PathValue, Phase, ValueKind};
    use pyrudder_core::Result;

    const TARGET: &str = r"C:\Users\owner\AppData\Local\Microsoft\WindowsApps";

    fn value(text: &str) -> PathValue {
        PathValue {
            kind: ValueKind::ExpandString,
            text: text.to_owned(),
        }
    }

    fn journal() -> Journal {
        let before = value(&format!(r"C:\Windows;{TARGET};C:\Tools"));
        Journal {
            schema_version: 1,
            owner_sid: "S-1-5-21-1-2-3-1001".into(),
            target: TARGET.into(),
            install_dir: r"D:\PyRudder\bin".into(),
            shims_dir: r"D:\PyRudder\shims".into(),
            after: before.without(TARGET),
            before,
            phase: Phase::Applied,
        }
    }

    #[test]
    fn exact_restoration_only_reinserts_the_target() -> Result<()> {
        let journal = journal();
        journal.validate(&journal.owner_sid, TARGET)?;
        assert_eq!(
            journal.restoration(Some(&journal.after))?,
            Some(journal.before.clone())
        );
        Ok(())
    }

    #[test]
    fn edited_path_keeps_all_current_entries_and_type() -> Result<()> {
        let journal = journal();
        let current = PathValue {
            kind: ValueKind::String,
            text: r"C:\New;C:\Windows;C:\Tools;C:\Other".into(),
        };
        let restored = journal.restoration(Some(&current))?;
        assert_eq!(
            restored,
            Some(PathValue {
                kind: ValueKind::String,
                text: format!(r"C:\New;C:\Windows;{TARGET};C:\Tools;C:\Other"),
            })
        );
        assert!(journal.restoration(Some(&journal.before))?.is_none());
        Ok(())
    }

    #[test]
    fn forged_snapshot_cannot_change_unrelated_entries() {
        let mut journal = journal();
        journal.after.text = r"C:\Injected".into();
        assert!(journal.validate(&journal.owner_sid, TARGET).is_err());
        let mut journal = self::journal();
        journal.target = r"C:\Different".into();
        assert!(journal.validate(&journal.owner_sid, TARGET).is_err());
    }

    #[test]
    fn ambiguous_pending_repair_is_not_restored() {
        let mut journal = journal();
        journal.phase = Phase::Pending;
        assert!(journal.restoration(Some(&value(r"C:\Changed"))).is_err());
    }

    #[test]
    fn other_users_and_variable_based_machine_entries_are_untouched() {
        let before = value(&format!(
            r"%LOCALAPPDATA%\Microsoft\WindowsApps;C:\Users\other\AppData\Local\Microsoft\WindowsApps;{TARGET};C:\Tools"
        ));
        assert_eq!(
            before.without(TARGET).text,
            r"%LOCALAPPDATA%\Microsoft\WindowsApps;C:\Users\other\AppData\Local\Microsoft\WindowsApps;C:\Tools"
        );
    }

    #[test]
    fn empty_path_components_survive_exact_restoration() -> Result<()> {
        for text in [
            format!(";{TARGET}"),
            format!("{TARGET};"),
            format!(";{TARGET};"),
        ] {
            let mut journal = journal();
            journal.before = value(&text);
            journal.after = journal.before.without(TARGET);
            journal.validate(&journal.owner_sid, TARGET)?;
            assert_eq!(
                journal.restoration(Some(&journal.after))?,
                Some(journal.before.clone())
            );
        }
        Ok(())
    }
}
