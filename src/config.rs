use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range, path::Path};

#[derive(Clone, Debug)]
pub struct Config {
    text: String,
    values: BTreeMap<String, (String, Range<usize>)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    pub key: String,
    pub before: String,
    pub after: String,
}

impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        Self::parse(
            std::fs::read_to_string(path).with_context(|| format!("Read {}", path.display()))?,
        )
    }

    pub fn parse(text: String) -> Result<Self> {
        let mut values = BTreeMap::new();
        let mut offset = 0;
        for raw in text.split_inclusive('\n') {
            let line = raw.trim_end_matches(['\r', '\n']);
            let line = if offset == 0 {
                line.trim_start_matches('\u{feff}')
            } else {
                line
            };
            let bom = if offset == 0 && text.starts_with('\u{feff}') {
                3
            } else {
                0
            };
            if let Some((left, right)) = line.split_once('=') {
                let key = left.trim();
                if !key.is_empty() && !key.starts_with('#') {
                    let trimmed = right.trim();
                    let leading = right.len() - right.trim_start().len();
                    let start = offset + bom + left.len() + 1 + leading;
                    let (value, range) = if trimmed.starts_with('"') {
                        ensure!(
                            trimmed.ends_with('"') && trimmed.len() >= 2,
                            "Malformed value for {key}"
                        );
                        (
                            &trimmed[1..trimmed.len() - 1],
                            start + 1..start + trimmed.len() - 1,
                        )
                    } else {
                        (trimmed, start..start + trimmed.len())
                    };
                    ensure!(!value.contains('"'), "Malformed value for {key}");
                    ensure!(
                        values
                            .insert(key.to_owned(), (value.to_owned(), range))
                            .is_none(),
                        "Duplicate config key: {key}"
                    );
                }
            }
            offset += raw.len();
        }
        Ok(Self { text, values })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|v| v.0.as_str())
    }
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }
    pub fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    pub fn edit(&self, edits: &[Edit], reverse: bool) -> Result<Vec<u8>> {
        let mut replacements = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for edit in edits {
            ensure!(seen.insert(&edit.key), "Duplicate edit: {}", edit.key);
            ensure!(
                !edit.after.contains(['\n', '\r', '"']),
                "Invalid config value"
            );
            let Some((value, range)) = self.values.get(&edit.key) else {
                bail!("Config key disappeared: {}", edit.key)
            };
            let (expected, replacement) = if reverse {
                (&edit.after, &edit.before)
            } else {
                (&edit.before, &edit.after)
            };
            // Restore is idempotent, and preserves unrelated changes made since the backup.
            if reverse && value == replacement {
                continue;
            }
            ensure!(
                value == expected,
                "{} changed since preview (expected {:?}, found {:?}); rescan first",
                edit.key,
                expected,
                value
            );
            replacements.push((range.clone(), replacement.as_str()));
        }
        replacements.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        let mut text = self.text.clone();
        for (range, value) in replacements {
            text.replace_range(range, value);
        }
        Ok(text.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_bytes_and_only_selected_instance() {
        let c = Config::parse(
            "# keep\r\nbst.instance.Pie64.x = \"1\"\r\nbst.instance.Pie64_1.x=\"1\"".into(),
        )
        .unwrap();
        let e = Edit {
            key: "bst.instance.Pie64.x".into(),
            before: "1".into(),
            after: "0".into(),
        };
        assert_eq!(
            c.edit(&[e], false).unwrap(),
            b"# keep\r\nbst.instance.Pie64.x = \"0\"\r\nbst.instance.Pie64_1.x=\"1\""
        );
    }
    #[test]
    fn rejects_ambiguous_config_and_stale_preview() {
        assert!(Config::parse("x=1\nx=0\n".into()).is_err());
        let c = Config::parse("x=2\n".into()).unwrap();
        assert!(
            c.edit(
                &[Edit {
                    key: "x".into(),
                    before: "1".into(),
                    after: "0".into()
                }],
                false
            )
            .is_err()
        );
    }
    #[test]
    fn restore_preserves_new_runtime_values() {
        let c = Config::parse("ad=0\nport=5565\n".into()).unwrap();
        let e = Edit {
            key: "ad".into(),
            before: "1".into(),
            after: "0".into(),
        };
        assert_eq!(c.edit(&[e], true).unwrap(), b"ad=1\nport=5565\n");
    }
    #[test]
    fn bom_and_unquoted_are_preserved() {
        let c = Config::parse("\u{feff}flag = true\n".into()).unwrap();
        assert_eq!(c.get("flag"), Some("true"));
        let e = Edit {
            key: "flag".into(),
            before: "true".into(),
            after: "false".into(),
        };
        assert_eq!(
            String::from_utf8(c.edit(&[e], false).unwrap()).unwrap(),
            "\u{feff}flag = false\n"
        );
    }
}
