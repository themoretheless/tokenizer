import { runProcess } from './server/process.js'
import { fileURLToPath, URL } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

const repositoryRoot = fileURLToPath(new URL('..', import.meta.url))

function runBridge(args, source, signal) {
  return runProcess('cargo',
    ['run', '--quiet', '--features', 'web-bridge,all-languages', '--bin', 'tokenizer-web-bridge', '--', ...args],
    { cwd: repositoryRoot, source: source ?? '', signal })
}

function tokenize(source, language, mode, layer) {
  return runBridge(['--language', language, '--mode', mode, '--layer', layer], source)
}

// The registry only changes when Rust code is rebuilt, so one cargo run per server.
let catalogPromise
function catalog() {
  catalogPromise ??= runBridge(['--catalog']).then((stdout) => stdout.trim())
  return catalogPromise
}

function rustBridge() {
  return {
    name: 'tokenizer-rust-bridge',
    configureServer(server) {
      server.middlewares.use('/api/catalog', async (request, response) => {
        if (request.method !== 'GET') { response.statusCode = 405; response.end('GET required'); return }
        try {
          response.setHeader('content-type', 'application/json; charset=utf-8')
          response.setHeader('cache-control', 'no-store')
          response.end(await catalog())
        } catch (error) {
          catalogPromise = undefined
          response.statusCode = 500
          response.setHeader('content-type', 'application/json; charset=utf-8')
          response.end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }))
        }
      })
      server.middlewares.use('/api/run-rush', async (request, response) => {
        if (request.method !== 'POST') { response.statusCode = 405; response.end('POST required'); return }
        const chunks = []
        let size = 0
        for await (const chunk of request) {
          size += chunk.length
          if (size > 1024 * 1024) { response.statusCode = 413; response.end('Source exceeds 1 MiB'); return }
          chunks.push(chunk)
        }
        response.setHeader('content-type', 'application/json; charset=utf-8')
        try {
          const payload = JSON.parse(Buffer.concat(chunks).toString('utf8'))
          if (typeof payload.source !== 'string') throw new Error('Source must be a string')
          const controller = new AbortController()
          const abort = () => { if (!response.writableEnded) controller.abort() }
          response.once('close', abort)
          if (response.destroyed) controller.abort()
          try {
            const result = await runBridge(['--run-rush'], payload.source, controller.signal)
            if (!response.destroyed) response.end(result)
          } finally { response.removeListener('close', abort) }
        } catch (error) {
          response.statusCode = 500
          response.end(JSON.stringify({ ok: false, error: String(error) }))
        }
      })
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
          const language =
            typeof payload.language === 'string' && payload.language.trim()
              ? payload.language.trim()
              : 'json'
          const mode =
            typeof payload.mode === 'string' && payload.mode.trim()
              ? payload.mode.trim()
              : language === 'json'
                ? 'strict'
                : 'default'
          const layer = payload.layer === 'syntax' ? 'syntax' : 'semantic'
          const result = await tokenize(source, language, mode, layer)
          response.setHeader('content-type', 'application/json; charset=utf-8')
          response.end(result)
        } catch (error) {
          response.statusCode = 500
          response.setHeader('content-type', 'application/json; charset=utf-8')
          response.end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }))
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
