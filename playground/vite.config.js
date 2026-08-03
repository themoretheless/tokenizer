import { spawn } from 'node:child_process'
import { fileURLToPath, URL } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

const repositoryRoot = fileURLToPath(new URL('..', import.meta.url))

function tokenize(source, mode, layer) {
  return new Promise((resolve, reject) => {
    const child = spawn('cargo', ['run', '--quiet', '--bin', 'tokenizer-web-bridge', '--', '--mode', mode, '--layer', layer], { cwd: repositoryRoot, stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8').on('data', (chunk) => { stdout += chunk })
    child.stderr.setEncoding('utf8').on('data', (chunk) => { stderr += chunk })
    child.on('error', reject)
    child.on('close', (code) => code === 0 ? resolve(stdout) : reject(new Error(stderr || `Rust bridge exited with code ${code}`)))
    child.stdin.end(source)
  })
}

function rustBridge() {
  return {
    name: 'tokenizer-rust-bridge',
    configureServer(server) {
      server.middlewares.use('/api/tokenize', async (request, response) => {
        if (request.method !== 'POST') { response.statusCode = 405; response.end('POST required'); return }
        const chunks = []
        let size = 0
        for await (const chunk of request) {
          size += chunk.length
          if (size > 1024 * 1024) { response.statusCode = 413; response.end('Source is limited to 1 MiB in the playground'); return }
          chunks.push(chunk)
        }
        try {
          const payload = JSON.parse(Buffer.concat(chunks).toString('utf8'))
          const source = typeof payload.source === 'string' ? payload.source : ''
          const mode = payload.mode === 'jsonc' ? 'jsonc' : 'strict'
          const layer = payload.layer === 'syntax' ? 'syntax' : 'semantic'
          const result = await tokenize(source, mode, layer)
          response.setHeader('content-type', 'application/json; charset=utf-8')
          response.end(result)
        } catch (error) {
          response.statusCode = 500
          response.setHeader('content-type', 'application/json; charset=utf-8')
          response.end(JSON.stringify({ error: error.message }))
        }
      })
    },
  }
}

export default defineConfig({
  base: process.env.GITHUB_ACTIONS ? '/tokenizer/' : '/',
  plugins: [vue(), rustBridge()],
  server: { port: 4173 },
})
