use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, IsTerminal as _, Write as _};
use std::path::Path;

use overseerd_tooling_schema::ToolingDocument;

use crate::cli::{InspectFilters, TerminalPolicy};

pub(crate) fn write_text_inspection(
    document: &ToolingDocument,
    filters: &InspectFilters,
    color: TerminalPolicy,
    pager: TerminalPolicy,
) -> io::Result<()> {
    let terminal = io::stdout().is_terminal();
    let color = policy_enabled(color, terminal)
        && !(color == TerminalPolicy::Auto && std::env::var_os("NO_COLOR").is_some());
    let page = policy_enabled(pager, terminal) && pager_command().is_some();

    if !page {
        return crate::render::write_inspection(document, filters, color, &mut io::stdout().lock());
    }

    let mut rendered = Vec::new();

    crate::render::write_inspection(document, filters, color, &mut rendered)?;
    write_to_pager(&rendered)
}

pub(crate) fn write_export(
    output: Option<&Path>,
    write: impl FnOnce(&mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()> {
    match output {
        Some(path) => write_export_file(path, write),
        None => write(&mut io::stdout().lock()),
    }
}

fn write_to_pager(rendered: &[u8]) -> io::Result<()> {
    let (program, arguments) = pager_command().ok_or_else(|| io::Error::other("pager disabled"))?;
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .env("LESS", "FRX")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("pager stdin was not piped"))?
        .write_all(rendered);
    let status = child.wait()?;

    if write_result
        .as_ref()
        .is_err_and(|error| error.kind() != io::ErrorKind::BrokenPipe)
    {
        return write_result;
    }

    if !status.success() {
        return Err(io::Error::other(format!(
            "pager exited with status {status}"
        )));
    }

    Ok(())
}

fn write_export_file(
    path: &Path,
    write: impl FnOnce(&mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::other("export path has no file name"))?;
    let mut temporary = None;

    for attempt in 0_u16..100 {
        let temporary_name = format!(
            ".{}.{}.{attempt}.tmp",
            file_name.to_string_lossy(),
            std::process::id()
        );
        let temporary_path = parent.join(temporary_name);

        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
        {
            Ok(file) => {
                temporary = Some((temporary_path, file));

                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    let (temporary_path, mut file) = temporary
        .ok_or_else(|| io::Error::other("could not create a unique temporary export file"))?;
    let permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let result = write(&mut file)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            if let Some(permissions) = permissions {
                std::fs::set_permissions(&temporary_path, permissions)?;
            }

            Ok(())
        })
        .and_then(|()| replace_file(&temporary_path, path));

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    result
}

fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if destination.exists() {
        return replace_file_windows(temporary, destination);
    }

    std::fs::rename(temporary, destination)
}

#[cfg(windows)]
fn replace_file_windows(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;

    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    // SAFETY: Both paths are valid null-terminated UTF-16 buffers for the duration of the call.
    let replaced = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };

    if replaced == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn pager_command() -> Option<(OsString, Vec<OsString>)> {
    let configured = std::env::var_os("PAGER");
    let command = configured.as_deref().and_then(|value| value.to_str());
    let command = command.unwrap_or("less").trim();

    if command.is_empty() || command == "cat" {
        return None;
    }

    let mut parts = split_shell_words(command).ok()?.into_iter();
    let program = parts.next()?;
    let arguments = parts.map(OsString::from).collect();

    Some((OsString::from(program), arguments))
}

fn split_shell_words(value: &str) -> Result<Vec<String>, ()> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = None;
    let mut escaped = false;

    for character in value.chars() {
        if escaped {
            word.push(character);
            escaped = false;

            continue;
        }

        match (quoted, character) {
            (Some('\''), '\'') | (Some('"'), '"') => quoted = None,
            (Some('\''), _) => word.push(character),
            (Some('"'), '\\') => word.push(character),
            (_, '\\') => escaped = true,
            (Some(_), _) => word.push(character),
            (None, '\'' | '"') => quoted = Some(character),
            (None, character) if character.is_ascii_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (None, _) => word.push(character),
        }
    }

    if escaped || quoted.is_some() {
        return Err(());
    }

    if !word.is_empty() {
        words.push(word);
    }

    Ok(words)
}

fn policy_enabled(policy: TerminalPolicy, terminal: bool) -> bool {
    match policy {
        TerminalPolicy::Auto => terminal,
        TerminalPolicy::Always => true,
        TerminalPolicy::Never => false,
    }
}

#[cfg(test)]
mod tests {
    use super::split_shell_words;

    #[test]
    fn pager_shell_words_preserve_quoted_paths_and_arguments() {
        assert_eq!(
            split_shell_words("'/Applications/My Pager.app/pager' -R 'wide output'")
                .expect("pager command parses"),
            ["/Applications/My Pager.app/pager", "-R", "wide output"]
        );
    }

    #[test]
    fn pager_shell_words_reject_incomplete_quotes() {
        assert!(split_shell_words("less 'unterminated").is_err());
    }

    #[test]
    fn pager_shell_words_preserve_windows_paths() {
        assert_eq!(
            split_shell_words(r#""C:\Program Files\Pager\pager.exe" -R"#)
                .expect("Windows pager command parses"),
            [r#"C:\Program Files\Pager\pager.exe"#, "-R"]
        );
    }
}
