use home::home_dir;
use libc::{tcgetattr, tcsetattr, termios, ECHO, ICANON, STDIN_FILENO, TCSANOW};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

mod shell;

static BUILTINS: [&str; 5] = ["cd", "pwd", "exit", "echo", "type"];

fn main() {
    let mut termios = unsafe { std::mem::zeroed::<termios>() };
    if unsafe { tcgetattr(STDIN_FILENO, &mut termios) } != 0 {
        eprintln!("sh: fatal error: {}", io::Error::last_os_error());
        process::exit(1);
    }

    termios.c_lflag &= !(ICANON | ECHO);

    if unsafe { tcsetattr(STDIN_FILENO, TCSANOW, &termios) } != 0 {
        eprintln!("sh: fatal error: {}", io::Error::last_os_error());
        process::exit(1);
    }

    let mut shell = shell::new();

    loop {
        shell.prompt();
        shell.process();
    }
}

fn resolve_path(path: &str) -> PathBuf {
    let path = Path::new(path);

    if path.starts_with("~") {
        if let Some(home) = home_dir() {
            if path == Path::new("~") {
                return home;
            } else {
                return home.join(path.strip_prefix("~").unwrap());
            }
        }
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    std::env::current_dir().unwrap().join(path)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn find_executable(target: &str) -> Option<String> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|path| {
            let full_path = path.join(target);
            if full_path.exists() && is_executable(&full_path) {
                Some(full_path.to_string_lossy().into_owned())
            } else {
                None
            }
        })
    })
}

fn load_executable() -> Vec<[String; 2]> {
    let mut results: Vec<[String; 2]> = vec![];

    if let Some(paths) = env::var_os("PATH") {
        for path in env::split_paths(&paths) {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    if let Ok(file_name) = entry.file_name().into_string() {
                        if is_executable(entry.path().as_path()) {
                            let full_path = entry.path();
                            if let Some(full_path_str) = full_path.to_str() {
                                results.push([file_name, full_path_str.to_string()]);
                            }
                        }
                    }
                }
            }
        }
    }

    results
}

fn get_completions<'a>(commands: &'a [[String; 2]], prefix: &str) -> Vec<&'a str> {
    let mut results: Vec<&str> = BUILTINS
        .iter()
        .filter(|cmd| cmd.starts_with(prefix))
        .cloned()
        .collect();

    let mut system_results = commands
        .iter()
        .filter(|cmd| cmd[0].starts_with(prefix))
        .map(|cmd| cmd[0].as_str())
        .collect();

    results.append(&mut system_results);
    results.sort();
    results.sort_by_key(|s| s.len());
    results
}
