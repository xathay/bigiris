//! XFCE Thunar — merges our actions into `~/.config/Thunar/uca.xml`,
//! preserving any custom actions the user added via Thunar's own GUI.
//!
//! Thunar stores every user custom action in one XML file with a fixed
//! shape produced by Thunar itself, so a careful state-machine walk over
//! the string beats parsing the full XML grammar. Our actions are all
//! tagged with `<unique-id>bigiris-…</unique-id>` and live as siblings of
//! any pre-existing `<action>` blocks; uninstall just drops ours.

use std::io;
use std::path::{Path, PathBuf};

use crate::action::{Action, ACTIONS, TOP_LEVEL_LABEL};
use crate::scope::ScopePaths;

const EMPTY_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n";

/// Install (or refresh) our actions in `~/.config/Thunar/uca.xml`.
pub fn install(paths: &ScopePaths) -> io::Result<Vec<PathBuf>> {
    let target = paths.thunar_uca();
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = read_or_default(&target)?;
    let stripped = strip_bigiris_actions(&existing);
    let merged = inject_before_close(&stripped, &render_all_actions());
    std::fs::write(&target, merged)?;
    Ok(vec![target])
}

/// Drop every bigiris-owned `<action>` from `uca.xml`. If the resulting file
/// contains no actions at all (we were the only source), it is removed so
/// Thunar falls back to its built-in defaults.
pub fn uninstall(paths: &ScopePaths) -> io::Result<Vec<PathBuf>> {
    let target = paths.thunar_uca();
    if !target.exists() {
        return Ok(Vec::new());
    }
    let existing = std::fs::read_to_string(&target)?;
    let stripped = strip_bigiris_actions(&existing);
    if !stripped.contains("<action>") {
        std::fs::remove_file(&target)?;
    } else {
        std::fs::write(&target, stripped)?;
    }
    Ok(vec![target])
}

fn read_or_default(path: &Path) -> io::Result<String> {
    if path.exists() {
        std::fs::read_to_string(path)
    } else {
        Ok(EMPTY_XML.to_string())
    }
}

/// Remove every `<action>…<unique-id>bigiris-…</unique-id>…</action>` block,
/// plus any leading whitespace so we don't leave orphan blank lines behind.
fn strip_bigiris_actions(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut cursor = 0;

    while let Some(rel) = xml[cursor..].find("<action>") {
        let action_start = cursor + rel;
        let Some(end_rel) = xml[action_start..].find("</action>") else {
            // Malformed input — stop stripping, preserve the rest verbatim.
            out.push_str(&xml[cursor..]);
            return out;
        };
        let action_end = action_start + end_rel + "</action>".len();
        let body = &xml[action_start..action_end];
        let is_ours = body.contains("<unique-id>bigiris-");

        if is_ours {
            // Rewind out of the leading whitespace that used to separate this
            // block from the previous line — we don't want to leave a gap.
            let rtrimmed_len = out.trim_end_matches([' ', '\t']).len();
            out.truncate(rtrimmed_len);
            if out.ends_with('\n') {
                // keep exactly one newline between surviving blocks
            }
            cursor = action_end;
            // Eat exactly one trailing newline so blocks don't accumulate gaps.
            if xml.as_bytes().get(cursor).copied() == Some(b'\n') {
                cursor += 1;
            }
        } else {
            out.push_str(&xml[cursor..action_end]);
            cursor = action_end;
        }
    }

    out.push_str(&xml[cursor..]);
    out
}

fn inject_before_close(xml: &str, new_actions: &str) -> String {
    match xml.rfind("</actions>") {
        Some(idx) => {
            let before = xml[..idx].trim_end().to_string();
            let after = &xml[idx..];
            let mut result = before;
            result.push('\n');
            result.push_str(new_actions);
            result.push('\n');
            result.push_str(after);
            result
        }
        None => {
            // No sensible root element — produce one from scratch.
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n{new_actions}\n</actions>\n"
            )
        }
    }
}

fn render_all_actions() -> String {
    ACTIONS.iter().map(render_action).collect::<Vec<_>>().join("\n")
}

fn render_action(action: &Action) -> String {
    let submenu_path = match action.submenu {
        Some(sub) => format!("{TOP_LEVEL_LABEL}/{}", sub.label),
        None => TOP_LEVEL_LABEL.to_string(),
    };
    format!(
        "<action>\n\
         \t<icon>{icon}</icon>\n\
         \t<name>{label}</name>\n\
         \t<submenu>{submenu}</submenu>\n\
         \t<unique-id>bigiris-{id}</unique-id>\n\
         \t<command>{cmd}</command>\n\
         \t<description>BigIris — {label}</description>\n\
         \t<range>0</range>\n\
         \t<patterns>*</patterns>\n\
         \t<image-files/>\n\
         </action>",
        icon = escape_xml(action.icon),
        label = escape_xml(action.label),
        submenu = escape_xml(&submenu_path),
        id = action.id,
        cmd = escape_xml(action.command),
    )
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn install_on_fresh_home_creates_file_with_only_our_actions() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        install(&paths).unwrap();
        let body = read(&paths.thunar_uca());
        for action in ACTIONS {
            assert!(
                body.contains(&format!("<unique-id>bigiris-{}</unique-id>", action.id)),
                "missing action {}",
                action.id
            );
        }
        assert!(body.starts_with("<?xml"));
        assert!(body.contains("<actions>"));
        assert!(body.contains("</actions>"));
    }

    #[test]
    fn install_preserves_existing_foreign_actions() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        std::fs::create_dir_all(paths.thunar_uca().parent().unwrap()).unwrap();
        let existing = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                        <actions>\n\
                        <action>\n\
                        \t<icon>utilities-terminal</icon>\n\
                        \t<name>Open Terminal Here</name>\n\
                        \t<unique-id>user-custom-1700000000</unique-id>\n\
                        \t<command>xterm</command>\n\
                        \t<description>Open a terminal</description>\n\
                        </action>\n\
                        </actions>\n";
        std::fs::write(paths.thunar_uca(), existing).unwrap();

        install(&paths).unwrap();
        let body = read(&paths.thunar_uca());
        assert!(body.contains("user-custom-1700000000"), "foreign action nao preservada");
        assert!(body.contains("bigiris-convert-png"), "nossa acao nao injetada");
    }

    #[test]
    fn install_is_idempotent_on_repeated_runs() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        install(&paths).unwrap();
        let first = read(&paths.thunar_uca());
        install(&paths).unwrap();
        let second = read(&paths.thunar_uca());
        let first_count = first.matches("bigiris-convert-png").count();
        let second_count = second.matches("bigiris-convert-png").count();
        assert_eq!(first_count, 1, "primeira execucao duplicou? ({first_count})");
        assert_eq!(second_count, 1, "segunda execucao duplicou? ({second_count})");
    }

    #[test]
    fn uninstall_drops_our_actions_only() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        std::fs::create_dir_all(paths.thunar_uca().parent().unwrap()).unwrap();
        let existing = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                        <actions>\n\
                        <action>\n\
                        \t<name>Mine</name>\n\
                        \t<unique-id>user-keep-me</unique-id>\n\
                        \t<command>echo</command>\n\
                        </action>\n\
                        </actions>\n";
        std::fs::write(paths.thunar_uca(), existing).unwrap();

        install(&paths).unwrap();
        uninstall(&paths).unwrap();
        let body = read(&paths.thunar_uca());
        assert!(body.contains("user-keep-me"));
        assert!(!body.contains("bigiris-"), "restou acao nossa apos uninstall");
    }

    #[test]
    fn uninstall_removes_file_when_only_ours_existed() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        install(&paths).unwrap();
        uninstall(&paths).unwrap();
        assert!(!paths.thunar_uca().exists(), "uca.xml nao deveria sobreviver");
    }

    #[test]
    fn uninstall_on_missing_file_is_noop() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        let out = uninstall(&paths).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn escape_xml_handles_reserved_chars() {
        assert_eq!(escape_xml("a & b < c"), "a &amp; b &lt; c");
        assert_eq!(escape_xml("\"hi\""), "&quot;hi&quot;");
    }

    #[test]
    fn strip_bigiris_handles_block_at_file_start() {
        let xml = "<actions>\n\
                   <action><unique-id>bigiris-test</unique-id></action>\n\
                   <action><unique-id>other</unique-id></action>\n\
                   </actions>\n";
        let stripped = strip_bigiris_actions(xml);
        assert!(!stripped.contains("bigiris-test"));
        assert!(stripped.contains("<unique-id>other</unique-id>"));
    }
}
