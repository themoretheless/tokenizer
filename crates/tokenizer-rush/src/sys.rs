//! Native system services. Importing `sys` in the CLI enables this module.
use crate::{CancellationToken, HostRegistration, Value, ValueType};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const MODULE_SOURCE: &str = include_str!("sys.r");
#[derive(Clone, Debug)]
pub struct SysLimits {
    pub bytes: usize,
    pub processes: usize,
    pub timeout: Duration,
}
impl Default for SysLimits {
    fn default() -> Self {
        Self {
            bytes: 1_048_576,
            processes: 32,
            timeout: Duration::from_secs(30),
        }
    }
}
#[derive(Clone)]
struct Context {
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    args: Vec<String>,
}
pub struct SysHost {
    pub registrations: Vec<HostRegistration>,
}
fn text<'a>(v: &'a Value<'_>) -> &'a str {
    let Value::String(s) = v else {
        unreachable!("validated host argument")
    };
    s
}
fn result<'s>(value: Result<Value<'s>, String>) -> Value<'s> {
    match value {
        Ok(v) => Value::Variant("Ok", vec![v]),
        Err(e) => Value::Variant("Err", vec![Value::String(e)]),
    }
}
impl SysHost {
    pub fn new(limits: SysLimits) -> Self {
        Self::with_args(limits, Vec::new())
    }
    pub fn with_args(limits: SysLimits, args: Vec<String>) -> Self {
        let context = Rc::new(RefCell::new(Context {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env: BTreeMap::new(),
            args,
        }));
        let mut registrations = Vec::new();
        let string = ValueType::String;
        let outcome = |ty| ValueType::Result(Box::new(ty), Box::new(ValueType::String));
        let list = ValueType::List(Box::new(string.clone()));
        let output = ValueType::Tuple(vec![ValueType::Number, string.clone(), string.clone()]);
        for (name, pipeline, configured) in [
            ("__sys_run", false, false),
            ("__sys_pipe", true, false),
            ("__sys_run_with", false, true),
            ("__sys_pipe_with", true, true),
        ] {
            let context = context.clone();
            let limits = limits.clone();
            let mut parameters = if pipeline {
                vec![ValueType::List(Box::new(list.clone())), string.clone()]
            } else {
                vec![string.clone(), list.clone(), string.clone()]
            };
            if configured {
                parameters.push(ValueType::Option(Box::new(string.clone())));
                parameters.push(ValueType::List(Box::new(ValueType::Tuple(vec![
                    string.clone(),
                    string.clone(),
                ]))));
            }
            registrations.push(HostRegistration::new(
                name,
                parameters,
                outcome(output.clone()),
                move |args, token| {
                    let commands: Vec<Vec<String>> = if pipeline {
                        let Value::List(commands) = &args[0] else {
                            unreachable!()
                        };
                        commands
                            .iter()
                            .map(|v| {
                                let Value::List(args) = v else { unreachable!() };
                                args.iter().map(|v| text(v).to_owned()).collect()
                            })
                            .collect()
                    } else {
                        let Value::List(arguments) = &args[1] else {
                            unreachable!()
                        };
                        vec![
                            std::iter::once(text(&args[0]).to_owned())
                                .chain(arguments.iter().map(|v| text(v).to_owned()))
                                .collect(),
                        ]
                    };
                    let input_index = if pipeline { 1 } else { 2 };
                    let input = text(&args[input_index]);
                    let mut context = context.borrow().clone();
                    if configured
                        && let Err(error) = configure(
                            &mut context,
                            &args[input_index + 1],
                            &args[input_index + 2],
                            limits.bytes,
                        )
                    {
                        return Ok(result(Err(error)));
                    }
                    Ok(result(
                        execute(&commands, input, &context, &limits, token).map(
                            |(code, out, err)| {
                                Value::Tuple(vec![
                                    Value::Number(code as f64),
                                    Value::String(out),
                                    Value::String(err),
                                ])
                            },
                        ),
                    ))
                },
            ));
        }
        for (name, kind, ty) in [
            ("__sys_read", 0, string.clone()),
            ("__sys_write", 1, ValueType::Bool),
            ("__sys_list", 2, list.clone()),
            ("__sys_mkdir", 3, ValueType::Bool),
            ("__sys_remove", 4, ValueType::Bool),
            ("__sys_cd", 5, ValueType::Bool),
            ("__sys_rmdir", 6, ValueType::Bool),
            ("__sys_exists", 7, ValueType::Bool),
        ] {
            let context = context.clone();
            let limit = limits.bytes;
            let parameters = if kind == 1 {
                vec![string.clone(), string.clone()]
            } else {
                vec![string.clone()]
            };
            registrations.push(HostRegistration::new(
                name,
                parameters,
                outcome(ty),
                move |args, _| {
                    let path = context.borrow().cwd.join(text(&args[0]));
                    let operation = (|| -> Result<Value<'_>, String> {
                        Ok(match kind {
                            0 => {
                                if !std::fs::metadata(&path)
                                    .map_err(|e| e.to_string())?
                                    .is_file()
                                {
                                    return Err(
                                        "read requires a regular file; use input for stdin".into(),
                                    );
                                }
                                let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                                let mut bytes = Vec::new();
                                file.take(limit.saturating_add(1) as u64)
                                    .read_to_end(&mut bytes)
                                    .map_err(|e| e.to_string())?;
                                if bytes.len() > limit {
                                    return Err("File byte limit exceeded".into());
                                };
                                Value::String(String::from_utf8(bytes).map_err(|e| e.to_string())?)
                            }
                            1 => {
                                let data = text(&args[1]);
                                if data.len() > limit {
                                    return Err("File byte limit exceeded".into());
                                };
                                std::fs::write(path, data).map_err(|e| e.to_string())?;
                                Value::Bool(true)
                            }
                            2 => {
                                let mut names = Vec::new();
                                let mut bytes = 0usize;
                                for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
                                    let name = entry
                                        .map_err(|e| e.to_string())?
                                        .file_name()
                                        .into_string()
                                        .map_err(|_| "Non-UTF-8 filename")?;
                                    bytes = bytes.saturating_add(name.len().saturating_add(1));
                                    if bytes > limit {
                                        return Err("Directory byte limit exceeded".into());
                                    };
                                    names.push(name);
                                }
                                names.sort();
                                Value::List(names.into_iter().map(Value::String).collect())
                            }
                            3 => {
                                std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
                                Value::Bool(true)
                            }
                            4 => {
                                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                                Value::Bool(true)
                            }
                            6 => {
                                std::fs::remove_dir(path).map_err(|e| e.to_string())?;
                                Value::Bool(true)
                            }
                            7 => Value::Bool(path.try_exists().map_err(|e| e.to_string())?),
                            _ => {
                                let path =
                                    std::fs::canonicalize(path).map_err(|e| e.to_string())?;
                                if !path.is_dir() {
                                    return Err("Working directory must be a directory".into());
                                };
                                context.borrow_mut().cwd = path;
                                Value::Bool(true)
                            }
                        })
                    })();
                    Ok(result(operation))
                },
            ));
        }
        let ctx = context.clone();
        registrations.push(HostRegistration::new(
            "__sys_args",
            vec![],
            list,
            move |_, _| {
                Ok(Value::List(
                    ctx.borrow()
                        .args
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ))
            },
        ));
        let bytes = limits.bytes;
        let timeout = limits.timeout;
        registrations.push(HostRegistration::new(
            "__sys_input",
            vec![],
            outcome(string.clone()),
            move |_, token| Ok(result(read_input(bytes, timeout, token).map(Value::String))),
        ));
        for (name, stderr) in [("__sys_out", false), ("__sys_err", true)] {
            registrations.push(HostRegistration::new(
                name,
                vec![string.clone()],
                outcome(ValueType::Bool),
                move |args, token| {
                    let data = text(&args[0]);
                    if data.len() > bytes {
                        return Ok(result(Err("Output byte limit exceeded".into())));
                    }
                    Ok(result(
                        write_stream(data.as_bytes(), stderr, timeout, token)
                            .map(|_| Value::Bool(true)),
                    ))
                },
            ));
        }
        let ctx = context.clone();
        registrations.push(HostRegistration::new(
            "__sys_env",
            vec![string.clone()],
            ValueType::Option(Box::new(string.clone())),
            move |args, _| {
                let key = text(&args[0]);
                let value = ctx
                    .borrow()
                    .env
                    .get(key)
                    .cloned()
                    .or_else(|| std::env::var(key).ok());
                Ok(match value {
                    Some(s) => Value::Variant("Some", vec![Value::String(s)]),
                    None => Value::Variant("None", vec![]),
                })
            },
        ));
        let ctx = context.clone();
        let limit = limits.bytes;
        registrations.push(HostRegistration::new(
            "__sys_set_env",
            vec![string.clone(), string.clone()],
            outcome(ValueType::Bool),
            move |args, _| {
                let (name, value) = (text(&args[0]), text(&args[1]));
                if name.is_empty()
                    || name.contains(['=', '\0'])
                    || value.contains('\0')
                    || name.len().saturating_add(value.len()) > limit
                {
                    return Ok(result(Err(
                        "Invalid or oversized environment variable".into()
                    )));
                }
                let mut ctx = ctx.borrow_mut();
                let bytes: usize = ctx
                    .env
                    .iter()
                    .filter(|(key, _)| key.as_str() != name)
                    .map(|(key, value)| key.len().saturating_add(value.len()))
                    .sum();
                if bytes.saturating_add(name.len()).saturating_add(value.len()) > limit {
                    return Ok(result(Err("Environment byte limit exceeded".into())));
                }
                ctx.env.insert(name.to_owned(), value.to_owned());
                Ok(result(Ok(Value::Bool(true))))
            },
        ));
        registrations.push(HostRegistration::new(
            "__sys_cwd",
            vec![],
            string,
            move |_, _| {
                Ok(Value::String(
                    context
                        .borrow()
                        .cwd
                        .to_str()
                        .ok_or("Non-UTF-8 working directory")?
                        .to_owned(),
                ))
            },
        ));
        Self { registrations }
    }
}
struct Children(Vec<Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            if child.try_wait().ok().flatten().is_none() {
                kill(child);
            }
        }
        for child in &mut self.0 {
            let _ = child.wait();
        }
    }
}
fn kill(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}
fn reader<R: Read + Send + 'static>(
    mut stream: R,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> thread::JoinHandle<Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let count = stream.read(&mut chunk).map_err(|e| e.to_string())?;
            if count == 0 {
                return Ok(bytes);
            };
            if bytes.len().saturating_add(count) > limit {
                overflow.store(true, Ordering::SeqCst);
                return Err("Process output byte limit exceeded".into());
            };
            bytes.extend_from_slice(&chunk[..count]);
        }
    })
}
fn execute(
    commands: &[Vec<String>],
    input: &str,
    context: &Context,
    limits: &SysLimits,
    token: &CancellationToken,
) -> Result<(i32, String, String), String> {
    if commands.is_empty()
        || commands.len() > limits.processes
        || commands.iter().any(|c| c.is_empty() || c[0].is_empty())
    {
        return Err("Empty pipeline or process count limit exceeded".into());
    }
    if input.len() > limits.bytes
        || commands.iter().flatten().map(String::len).sum::<usize>() > limits.bytes
    {
        return Err("Process input byte limit exceeded".into());
    }
    if token.is_cancelled() {
        return Err("Execution cancelled".into());
    }
    let start = Instant::now();
    let overflow = Arc::new(AtomicBool::new(false));
    let mut children = Children(Vec::new());
    let mut errors = Vec::new();
    let mut previous = None;
    let mut stdin = None;
    for command in commands {
        if token.is_cancelled() {
            return Err("Execution cancelled".into());
        }
        if start.elapsed() >= limits.timeout {
            return Err("Process timeout exceeded".into());
        }
        let mut process = Command::new(&command[0]);
        process
            .args(&command[1..])
            .current_dir(&context.cwd)
            .envs(&context.env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        process.stdin(match previous.take() {
            Some(pipe) => Stdio::from(pipe),
            None => Stdio::piped(),
        });
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.process_group(0);
        }
        let mut child = process.spawn().map_err(|e| e.to_string())?;
        if children.0.is_empty() {
            stdin = child.stdin.take();
        }
        previous = child.stdout.take();
        errors.push(reader(
            child.stderr.take().unwrap(),
            limits.bytes,
            overflow.clone(),
        ));
        children.0.push(child);
    }
    let out = reader(previous.unwrap(), limits.bytes, overflow.clone());
    let input = input.as_bytes().to_vec();
    let writer = thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            stdin.write_all(&input)
        } else {
            Ok(())
        }
    });
    let mut statuses = vec![None; children.0.len()];
    let abort = loop {
        if token.is_cancelled() {
            break Some("Execution cancelled");
        }
        if overflow.load(Ordering::SeqCst) {
            break Some("Process output byte limit exceeded");
        }
        if start.elapsed() >= limits.timeout {
            break Some("Process timeout exceeded");
        }
        for (child, status) in children.0.iter_mut().zip(&mut statuses) {
            if status.is_none() {
                *status = child.try_wait().map_err(|e| e.to_string())?;
            }
        }
        if statuses.iter().all(Option::is_some)
            && out.is_finished()
            && errors.iter().all(thread::JoinHandle::is_finished)
            && writer.is_finished()
        {
            break None;
        }
        thread::sleep(Duration::from_millis(5));
    };
    if abort.is_some() {
        for child in &mut children.0 {
            kill(child);
        }
    }
    let output = out.join().map_err(|_| "Process output reader failed")?;
    let mut stderr = Vec::new();
    for reader in errors {
        let bytes = reader.join().map_err(|_| "Process error reader failed")??;
        if stderr.len().saturating_add(bytes.len()) > limits.bytes {
            return Err("Process error byte limit exceeded".into());
        };
        stderr.extend(bytes);
    }
    let written = writer.join().map_err(|_| "Process input writer failed")?;
    if let Some(reason) = abort {
        return Err(reason.into());
    }
    // An early stdin close is normal for programs that do not consume input.
    if let Err(e) = written
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e.to_string());
    }
    let status = statuses
        .iter()
        .rev()
        .find_map(|s| s.filter(|s| !s.success()))
        .unwrap_or(statuses.last().unwrap().unwrap());
    #[cfg(unix)]
    let code = {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| -status.signal().unwrap_or(1))
    };
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(-1);
    Ok((
        code,
        String::from_utf8(output?).map_err(|e| e.to_string())?,
        String::from_utf8(stderr).map_err(|e| e.to_string())?,
    ))
}
fn read_input(
    limit: usize,
    timeout: Duration,
    token: &CancellationToken,
) -> Result<String, String> {
    #[cfg(unix)]
    {
        let start = Instant::now();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            if token.is_cancelled() {
                return Err("Execution cancelled".into());
            }
            if start.elapsed() >= timeout {
                return Err("Standard input timeout exceeded".into());
            }
            let mut fd = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            // poll/read borrow the existing stdin descriptor without taking ownership.
            let ready = unsafe { libc::poll(&mut fd, 1, 50) };
            if ready < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                };
                return Err(e.to_string());
            }
            if ready == 0 {
                continue;
            }
            let count =
                unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
            if count < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                };
                return Err(e.to_string());
            }
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count as usize) > limit {
                return Err("Standard input byte limit exceeded".into());
            }
            bytes.extend_from_slice(&buffer[..count as usize]);
        }
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = (timeout, token);
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(limit.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > limit {
            return Err("Standard input byte limit exceeded".into());
        }
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }
}
fn write_stream(
    data: &[u8],
    stderr: bool,
    timeout: Duration,
    token: &CancellationToken,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        let fd = if stderr {
            libc::STDERR_FILENO
        } else {
            libc::STDOUT_FILENO
        };
        let start = Instant::now();
        let mut offset = 0;
        while offset < data.len() {
            if token.is_cancelled() {
                return Err("Execution cancelled".into());
            }
            if start.elapsed() >= timeout {
                return Err("Standard output timeout exceeded".into());
            }
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, 50) };
            if ready < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.to_string());
            }
            if ready == 0 {
                continue;
            }
            // Use at most the POSIX minimum PIPE_BUF after readiness, so a single
            // CLI writer cannot block on a partially available pipe buffer.
            let count = (data.len() - offset).min(512);
            let written = unsafe { libc::write(fd, data[offset..].as_ptr().cast(), count) };
            if written < 0 {
                let e = std::io::Error::last_os_error();
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(e.to_string());
            }
            if written == 0 {
                return Err("Cannot write standard output".into());
            }
            offset += written as usize;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (timeout, token);
        let result = if stderr {
            std::io::stderr().write_all(data)
        } else {
            std::io::stdout().write_all(data)
        };
        result.map_err(|e| e.to_string())
    }
}

fn configure(
    context: &mut Context,
    cwd: &Value<'_>,
    environment: &Value<'_>,
    limit: usize,
) -> Result<(), String> {
    match cwd {
        Value::Variant("Some", values) => {
            let path = text(&values[0]);
            if path.len() > limit || path.contains('\0') {
                return Err("Invalid or oversized working directory".into());
            }
            context.cwd = context.cwd.join(path);
        }
        Value::Variant("None", _) => (),
        _ => unreachable!("validated working directory"),
    }
    let Value::List(entries) = environment else {
        unreachable!("validated environment")
    };
    let mut bytes = context.env.iter().fold(0usize, |sum, (key, value)| {
        sum.saturating_add(key.len()).saturating_add(value.len())
    });
    for entry in entries {
        let Value::Tuple(pair) = entry else {
            unreachable!("validated environment entry")
        };
        let (name, value) = (text(&pair[0]), text(&pair[1]));
        if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
            return Err("Invalid environment variable".into());
        }
        if let Some(previous) = context.env.get(name) {
            bytes = bytes.saturating_sub(name.len().saturating_add(previous.len()));
        }
        bytes = bytes.saturating_add(name.len()).saturating_add(value.len());
        if bytes > limit {
            return Err("Environment byte limit exceeded".into());
        }
        context.env.insert(name.to_owned(), value.to_owned());
    }
    Ok(())
}
