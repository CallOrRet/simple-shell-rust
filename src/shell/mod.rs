use std::env;
use std::fs::File;
use std::io::{self, Read, Write};
use std::process::{self, Command, Stdio};

use crate::{find_executable, get_completions, load_executable, resolve_path, BUILTINS};

pub struct Shell<'a> {
    stdin: io::StdinLock<'a>,
    stdout: io::StdoutLock<'a>,
    stderr: io::StderrLock<'a>,
    payload: String,
    executables: Vec<[String; 2]>,
}

pub fn new<'a>() -> Shell<'a> {
    Shell {
        stdin: io::stdin().lock(),
        stdout: io::stdout().lock(),
        stderr: io::stderr().lock(),
        payload: String::new(),
        executables: load_executable(),
    }
}

impl Shell<'_> {
    pub fn process(&mut self) -> i32 {
        let result = self.read_input();

        if result.is_none() {
            return 0;
        }

        let inputs = Shell::split_input(result.unwrap().as_str());

        if inputs.is_empty() {
            return 0;
        }

        let mut argv: Vec<&str> = vec![];
        let mut arg_iter = inputs.iter();
        let mut rstdout = "1";
        let mut rstdout_truncate = true;
        let mut rstderr = "2";
        let mut rstderr_truncate = true;

        let command = arg_iter.next().unwrap().as_str();

        while let Some(arg) = arg_iter.next() {
            if !arg.contains('>') {
                argv.push(arg);
                continue;
            }

            let mut truncate = true;

            let (mut from, mut to) = arg.split_once('>').unwrap();

            if to.starts_with('>') {
                to = &to[1..];
                truncate = false;
            }

            if from.is_empty() {
                from = "1";
            }

            if to.is_empty() {
                if let Some(arg) = arg_iter.next() {
                    to = arg;
                } else {
                    self.error("sh: Redirection expected a string, but found end of the input");
                    return 1;
                }
            }

            if from != "1" && from != "2" {
                continue;
            }

            if from == "1" {
                rstdout_truncate = truncate;
            } else {
                rstderr_truncate = truncate;
            }

            if let Some(fd) = to.strip_prefix("&") {
                if fd != "1" && fd != "2" {
                    self.error("sh: Bad file descriptor");
                    return 1;
                }
                if from == "1" && fd == "2" {
                    rstdout = rstderr;
                } else if from == "2" && fd == "1" {
                    rstderr = rstdout;
                }
            } else if from == "1" {
                rstdout = to;
            } else {
                rstderr = to;
            }
        }

        self.execute(
            command,
            argv,
            [rstdout, rstderr],
            [rstdout_truncate, rstderr_truncate],
        )
    }

    fn error<S: AsRef<str>>(&mut self, data: S) {
        self.stderr
            .write_all(data.as_ref().as_bytes())
            .unwrap_or_default();
        self.stderr.flush().unwrap_or_default();
    }

    fn output<S: AsRef<str>>(&mut self, data: S) {
        self.stdout
            .write_all(data.as_ref().as_bytes())
            .unwrap_or_default();
        self.stdout.flush().unwrap_or_default();
    }

    fn output_file<S: AsRef<str>>(&self, file: Option<&mut File>, data: S) {
        if let Some(file) = file {
            file.write_all(data.as_ref().as_bytes()).unwrap_or_default();
            file.flush().unwrap_or_default();
        }
    }

    pub fn prompt(&mut self) {
        self.output("$ ");
        if !self.payload.is_empty() {
            self.output(self.payload.clone());
        }
    }

    fn execute(
        &mut self,
        command: &str,
        argv: Vec<&str>,
        redirect: [&str; 2],
        truncate: [bool; 2],
    ) -> i32 {
        let mut stdout_file = if redirect[0] != "1" && redirect[0] != "2" {
            match File::options()
                .write(true)
                .create(true)
                .append(!truncate[0])
                .truncate(truncate[0])
                .open(redirect[0])
            {
                Ok(file) => Some(file),
                Err(error) => {
                    self.error(format!(
                        "sh: An error occurred while redirecting file {}, error: {}",
                        redirect[0], error
                    ));
                    return 1;
                }
            }
        } else {
            None
        };

        let mut stderr_file = if redirect[1] != "1" && redirect[1] != "2" {
            match File::options()
                .write(true)
                .create(true)
                .append(!truncate[1])
                .truncate(truncate[1])
                .open(redirect[1])
            {
                Ok(file) => Some(file),
                Err(error) => {
                    self.error(format!(
                        "sh: An error occurred while redirecting file {}, error: {}",
                        redirect[0], error
                    ));
                    return 1;
                }
            }
        } else {
            None
        };

        match command {
            "cd" => {
                let mut target = "~";

                if !argv.is_empty() {
                    target = argv.first().unwrap().to_owned();
                }

                let path = resolve_path(target);

                if path.is_dir() {
                    if env::set_current_dir(&path).is_err() {
                        self.error("cd: Failed to change working directory\n");
                        return 1;
                    }
                } else if path.is_file() {
                    self.error(format!("cd: {}: Is not a directory\n", target));
                    return 1;
                } else {
                    self.error(format!("cd: {}: No such file or directory\n", target));
                    return 1;
                }

                0
            }
            "pwd" => match env::current_dir() {
                Ok(path) => {
                    let result = format!("{}\n", path.display());

                    if stdout_file.is_some() {
                        self.output_file(stdout_file.as_mut(), result);
                    } else {
                        self.output(result);
                    }

                    0
                }
                Err(error) => {
                    let error = format!("pwd: {}\n", error);

                    if stderr_file.is_some() {
                        self.output_file(stderr_file.as_mut(), error);
                    } else {
                        self.error(error);
                    }

                    1
                }
            },
            "exit" => {
                if argv.is_empty() {
                    process::exit(0);
                } else {
                    process::exit(argv.first().unwrap().parse().unwrap_or(0));
                }
            }
            "echo" => {
                let result = format!("{} \n", argv.join(" "));

                if stdout_file.is_some() {
                    self.output_file(stdout_file.as_mut(), result);
                } else {
                    self.output(result);
                }

                0
            }
            "type" => {
                let mut status = 0;

                for target in argv.iter() {
                    let result = if BUILTINS.contains(target) {
                        format!("{} {}\n", target, "is a shell builtin")
                    } else {
                        match find_executable(target) {
                            Some(path) => {
                                status = 0;
                                format!("{}\n", path)
                            }
                            None => {
                                status = 1;
                                format!("{}: {}\n", target, "not found")
                            }
                        }
                    };

                    if stdout_file.is_some() {
                        self.output_file(stdout_file.as_mut(), result);
                    } else {
                        self.output(result);
                    }
                }

                status
            }
            _ => match find_executable(command) {
                Some(_) => {
                    let rstdout = match redirect[0] {
                        "1" => Stdio::from(io::stdout()),
                        "2" => Stdio::from(io::stderr()),
                        _ => Stdio::from(stdout_file.take().unwrap()),
                    };

                    let rstderr = match redirect[1] {
                        "1" => Stdio::from(io::stdout()),
                        "2" => Stdio::from(io::stderr()),
                        _ => Stdio::from(stderr_file.take().unwrap()),
                    };

                    match Command::new(command)
                        .args(argv)
                        .stdout(rstdout)
                        .stderr(rstderr)
                        .spawn()
                    {
                        Ok(mut child) => {
                            child.wait().unwrap_or_default().code().unwrap_or_default()
                        }
                        Err(error) => {
                            self.error(format!(
                                "sh: failed to execute command, error: {}\n",
                                error
                            ));
                            1
                        }
                    }
                }
                None => {
                    self.error(format!("{}: command not found\n", command));
                    1
                }
            },
        }
    }

    fn read_input(&mut self) -> Option<String> {
        let mut buffer = Vec::new();
        let mut remaining = Vec::new();
        let mut temp_buffer = [0u8; 1024];
        let mut completion_result: Option<String> = None;

        loop {
            if !remaining.is_empty() {
                buffer.extend_from_slice(&remaining);
                remaining.clear();
            }

            match self.stdin.read(&mut temp_buffer) {
                Ok(bytes_read) => {
                    if bytes_read == 0 {
                        return None;
                    }
                    buffer.extend_from_slice(&temp_buffer[..bytes_read]);
                }
                Err(error) => {
                    self.error(format!("sh: failed to read input, error: {}", error));
                    return None;
                }
            }

            let chars = match std::str::from_utf8(&buffer) {
                Ok(valid) => valid.chars(),
                Err(error) => {
                    let valid = error.valid_up_to();
                    remaining.extend_from_slice(&buffer[valid..]);
                    unsafe { std::str::from_utf8_unchecked(&buffer[..valid]).chars() }
                }
            };

            for ch in chars {
                if !ch.is_control() {
                    self.payload.push(ch);
                    self.output(ch.to_string());
                }

                if completion_result.is_some() {
                    if ch == '\t' {
                        self.output(completion_result.unwrap());
                        return None;
                    }
                    completion_result = None;
                }

                match ch {
                    '\t' => {
                        if !self.payload.is_empty() && !self.payload.ends_with(' ') {
                            let current_command = if let Some(pos) = self.payload.rfind(' ') {
                                &self.payload[pos + 1..]
                            } else {
                                self.payload.as_str()
                            };

                            if !current_command.is_empty() {
                                let completions =
                                    get_completions(&self.executables, current_command);

                                if completions.is_empty() {
                                    self.output("\x07");
                                    continue;
                                }

                                if completions.len() == 1 {
                                    let completion = completions[0];
                                    if completion.len() > current_command.len() {
                                        let result =
                                            &format!("{} ", &completion[current_command.len()..]);
                                        self.payload.push_str(result);
                                        self.output(result);
                                    } else {
                                        self.payload.push(' ');
                                        self.output(" ");
                                    }
                                } else {
                                    let same_length = if let Some(c) = completions.first() {
                                        let mut same = true;
                                        let length = c.len();
                                        for c in completions.iter().skip(1) {
                                            if c.len() != length {
                                                same = false;
                                                break;
                                            }
                                        }
                                        same
                                    } else {
                                        true
                                    };
                                    if same_length {
                                        completion_result =
                                            Some(format!("\n{}\n", completions.join("  ")));
                                        self.output("\x07");
                                    } else {
                                        let completion = completions.last().unwrap();
                                        if completion.len() > current_command.len() {
                                            let remain =
                                                completion.strip_prefix(current_command).unwrap();
                                            if let Some(index) = remain.find('_') {
                                                let result = &remain[..index].to_owned();
                                                self.payload.push_str(result);
                                                self.output(result);
                                            } else {
                                                let result = &format!("{} ", &remain);
                                                self.payload.push_str(result);
                                                self.output(result);
                                            }
                                        } else {
                                            self.payload.push(' ');
                                            self.output(" ");
                                        }
                                    }
                                }
                            }
                        } else {
                            // TODO:full completion
                        }
                    }
                    '\n' => {
                        self.output("\n");
                        if self.quotes_closed() {
                            return Some(std::mem::take(&mut self.payload));
                        } else {
                            self.payload.push(ch);
                        }
                    }
                    '\x08' | '\x7f' => {
                        if let Some(ch) = self.payload.pop() {
                            if ch == '\n' {
                                let mut count = 3;
                                if let Some(index) = self.payload.rfind('\n') {
                                    count = self.payload.len() - index;
                                } else {
                                    count += self.payload.len();
                                }
                                self.output(format!("\x1B[A\x1B[{}G", count));
                            } else if ch.len_utf8() > 1 {
                                self.output("\x1B[2D\x1B[2X");
                            } else {
                                self.output("\x1B[1D\x1B[1X");
                            }
                        }
                    }
                    _ => {}
                }
            }

            buffer.clear();
        }
    }

    fn split_input(input: &str) -> Vec<String> {
        let chars = input.chars();
        let mut escape = false;
        let mut payload = String::new();
        let mut in_quotes = false;
        let mut in_doublequotes = false;

        let mut inputs: Vec<String> = vec![];

        for ch in chars {
            if ch == '\\' && !escape {
                if in_quotes {
                    payload.push(ch);
                } else {
                    escape = true;
                }

                continue;
            }

            if escape {
                if in_doublequotes {
                    match ch {
                        '\\' | '\"' => {}
                        _ => {
                            payload.push('\\');
                        }
                    }
                }
                payload.push(ch);
                escape = false;
                continue;
            }

            match ch {
                '\"' if !in_quotes => {
                    in_doublequotes = !in_doublequotes;
                }
                '\'' if !in_doublequotes => {
                    in_quotes = !in_quotes;
                }
                ' ' if !in_quotes && !in_doublequotes => {
                    if !payload.is_empty() {
                        inputs.push(std::mem::take(&mut payload));
                    }
                }
                _ => {
                    payload.push(ch);
                }
            }
        }

        if !payload.is_empty() {
            inputs.push(std::mem::take(&mut payload));
        }

        inputs
    }

    fn quotes_closed(&self) -> bool {
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        let mut escaped = false;

        for c in self.payload.chars() {
            if escaped {
                escaped = false;
                continue;
            }

            match c {
                '\\' => {
                    escaped = true;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                }
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                }
                _ => {}
            }
        }

        !in_double_quote && !in_single_quote
    }
}
