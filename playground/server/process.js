import { spawn } from 'node:child_process'

export function runProcess(command, args, { cwd, source = '', signal } = {}) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(new Error('Execution cancelled')); return }
    const grouped = process.platform !== 'win32'
    const child = spawn(command, args, { cwd, detached: grouped, stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = '', stderr = '', cancelled = false
    function abort() {
      cancelled = true
      if (!child.pid) return
      try {
        // Only target the fresh process group created by this spawn.
        if (grouped) process.kill(-child.pid, 'SIGKILL')
        else child.kill('SIGKILL')
      } catch (error) { if (error.code !== 'ESRCH') reject(error) }
    }
    signal?.addEventListener('abort', abort, {once:true})
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk })
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk })
    child.stdin.on('error', error => { if (!cancelled && error.code !== 'EPIPE') reject(error) })
    child.on('error', error => { signal?.removeEventListener('abort', abort); reject(error) })
    child.on('close', code => {
      signal?.removeEventListener('abort', abort)
      if (cancelled) reject(new Error('Execution cancelled'))
      else if (code === 0) resolve(stdout)
      else reject(new Error(stderr || `Process exited with code ${code}`))
    })
    child.stdin.end(source)
  })
}
