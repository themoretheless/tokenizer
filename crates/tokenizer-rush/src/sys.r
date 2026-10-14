struct ProcessOutput { code: number, stdout: str, stderr: str }
struct CommandOptions { cwd: Option[str], env: list[tuple[str, str]] }
fn command_options() -> CommandOptions { return CommandOptions({cwd: None(), env: []}) }
fn process_output(value: tuple[number, str, str]) -> ProcessOutput {
    let (code, stdout, stderr) = value
    return ProcessOutput({code: code, stdout: stdout, stderr: stderr})
}
fn exec_with(program: str, args: list[str], input: str, options: CommandOptions) -> Result[ProcessOutput, str] {
    return Ok(process_output(__sys_run_with(program, args, input, options.cwd, options.env)?))
}
fn pipeline_with(commands: list[list[str]], input: str, options: CommandOptions) -> Result[ProcessOutput, str] {
    return Ok(process_output(__sys_pipe_with(commands, input, options.cwd, options.env)?))
}
fn exec(program: str, args: list[str], input: str) -> Result[ProcessOutput, str] {
    return Ok(process_output(__sys_run(program, args, input)?))
}
fn pipeline(commands: list[list[str]], input: str) -> Result[ProcessOutput, str] {
    return Ok(process_output(__sys_pipe(commands, input)?))
}
fn checked(output: ProcessOutput) -> Result[ProcessOutput, ProcessOutput] {
    if output.code == 0 { return Ok(output) }
    return Err(output)
}
fn run(program: str, args: list[str], input: str) -> Result[tuple[number, str, str], str] { return __sys_run(program, args, input) }
fn pipe(commands: list[list[str]], input: str) -> Result[tuple[number, str, str], str] { return __sys_pipe(commands, input) }
fn read(path: str) -> Result[str, str] { return __sys_read(path) }
fn write(path: str, data: str) -> Result[bool, str] { return __sys_write(path, data) }
fn list_dir(path: str) -> Result[list[str], str] { return __sys_list(path) }
fn mkdir(path: str) -> Result[bool, str] { return __sys_mkdir(path) }
fn remove(path: str) -> Result[bool, str] { return __sys_remove(path) }
fn cd(path: str) -> Result[bool, str] { return __sys_cd(path) }
fn env(name: str) -> Option[str] { return __sys_env(name) }
fn set_env(name: str, value: str) -> Result[bool, str] { return __sys_set_env(name, value) }
fn cwd() -> str { return __sys_cwd() }
fn args() -> list[str] { return __sys_args() }
fn input() -> Result[str, str] { return __sys_input() }
fn out(data: str) -> Result[bool, str] { return __sys_out(data) }
fn err(data: str) -> Result[bool, str] { return __sys_err(data) }
fn rmdir(path: str) -> Result[bool, str] { return __sys_rmdir(path) }
fn exists(path: str) -> Result[bool, str] { return __sys_exists(path) }
export CommandOptions, command_options, exec_with, pipeline_with, ProcessOutput, exec, pipeline, checked, rmdir, exists, args, input, out, err, run, pipe, read, write, list_dir, mkdir, remove, cd, env, set_env, cwd
