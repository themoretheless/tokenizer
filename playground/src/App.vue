<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { runTokenizer } from './tokenizer.js'

const examples = {
  valid: `{
  "project": "tokenizer",
  "unicode": "Тбилиси 😀",
  "stable": true,
  "versions": [1, 2, 3]
}`,
  recovery: `{
  "ready": true,
  "leadingZero": 01,
  "broken": "escape\\q",
  "missing":
}`,
  jsonc: `\uFEFF{
  // Comments and trailing commas are valid in JSONC
  "theme": "night",
  "features": ["tokens", "diagnostics",],
}`,
}

const source = ref(examples.valid)
const mode = ref('strict')
const layer = ref('semantic')
const result = ref(null)
const loading = ref(false)
const error = ref('')
const activeToken = ref(null)
let requestId = 0
let timer

const visibleTokens = computed(() => result.value?.tokens ?? [])
const diagnostics = computed(() => result.value?.diagnostics ?? [])
const active = computed(() => visibleTokens.value.find((token) => token.index === activeToken.value))

async function tokenize() {
  const id = ++requestId
  loading.value = true
  error.value = ''
  try {
    const payload = await runTokenizer({ source: source.value, mode: mode.value, layer: layer.value })
    if (id === requestId) result.value = payload
  } catch (cause) {
    if (id === requestId) {
      result.value = null
      error.value = cause.message
    }
  } finally {
    if (id === requestId) loading.value = false
  }
}

function loadExample(name) {
  source.value = examples[name]
  mode.value = name === 'jsonc' ? 'jsonc' : 'strict'
}

function focusSpan(start, end) {
  const token = visibleTokens.value.find((item) => item.start < Math.max(end, start + 1) && item.end > start)
  activeToken.value = token?.index ?? null
}

watch([source, mode, layer], () => {
  clearTimeout(timer)
  timer = setTimeout(tokenize, 180)
}, { immediate: true })
onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <main class="shell">
    <header class="topbar">
      <div>
        <p class="eyebrow">THEMORETHELESS / DEV TOOL</p>
        <h1>Tokenizer <i>Lab</i></h1>
      </div>
      <div class="status" :class="{ bad: result && !result.valid }">
        <span class="pulse" />
        {{ loading ? 'ANALYZING' : result?.valid ? 'VALID' : 'RECOVERED' }}
      </div>
    </header>

    <section class="controls">
      <div class="segmented" aria-label="JSON mode">
        <button :class="{ selected: mode === 'strict' }" @click="mode = 'strict'">Strict JSON</button>
        <button :class="{ selected: mode === 'jsonc' }" @click="mode = 'jsonc'">JSONC</button>
      </div>
      <div class="segmented" aria-label="Token layer">
        <button :class="{ selected: layer === 'semantic' }" @click="layer = 'semantic'">Semantic</button>
        <button :class="{ selected: layer === 'syntax' }" @click="layer = 'syntax'">Syntax</button>
      </div>
      <div class="examples">
        <span>Load</span>
        <button @click="loadExample('valid')">Valid</button>
        <button @click="loadExample('recovery')">Recovery</button>
        <button @click="loadExample('jsonc')">JSONC</button>
      </div>
    </section>

    <section class="workspace">
      <article class="panel editor-panel">
        <div class="panel-head"><span>01 / INPUT</span><span>{{ result?.sourceBytes ?? 0 }} UTF-8 BYTES</span></div>
        <textarea v-model="source" spellcheck="false" aria-label="JSON source" />
      </article>

      <article class="panel output-panel">
        <div class="panel-head"><span>02 / TOKEN MAP</span><span>{{ visibleTokens.length }} TOKENS</span></div>
        <pre class="highlight" aria-live="polite"><span
          v-for="token in visibleTokens"
          :key="token.index"
          :class="['token', `token-${token.kind}`, { active: activeToken === token.index }]"
          :title="`${token.kind} · ${token.start}..${token.end}`"
          @mouseenter="activeToken = token.index"
          @mouseleave="activeToken = null"
          @click="activeToken = token.index"
        >{{ token.text }}</span><span v-if="!visibleTokens.length" class="empty">Waiting for input…</span></pre>
        <div v-if="active" class="inspector"><b>{{ active.kind }}</b><code>{{ active.start }}..{{ active.end }}</code><span>{{ active.end - active.start }} bytes</span></div>
      </article>
    </section>

    <p v-if="error" class="bridge-error">{{ error }}</p>

    <section class="lower-grid">
      <article class="panel token-list">
        <div class="panel-head"><span>03 / TOKENS</span><span>BYTE SPANS</span></div>
        <div class="table-wrap">
          <table>
            <thead><tr><th>#</th><th>Kind</th><th>Span</th><th>Text</th></tr></thead>
            <tbody>
              <tr v-for="token in visibleTokens" :key="token.index" :class="{ active: activeToken === token.index }" @mouseenter="activeToken = token.index" @mouseleave="activeToken = null">
                <td>{{ String(token.index + 1).padStart(2, '0') }}</td>
                <td><span :class="['kind-dot', `bg-${token.kind}`]" />{{ token.kind }}</td>
                <td><code>{{ token.start }}..{{ token.end }}</code></td>
                <td><code>{{ JSON.stringify(token.text) }}</code></td>
              </tr>
            </tbody>
          </table>
        </div>
      </article>

      <article class="panel diagnostics">
        <div class="panel-head"><span>04 / DIAGNOSTICS</span><span>{{ diagnostics.length }}</span></div>
        <div v-if="!diagnostics.length" class="all-clear"><span>✓</span><b>No diagnostics</b><small>The input is valid in this mode.</small></div>
        <button v-for="diagnostic in diagnostics" :key="`${diagnostic.code}-${diagnostic.start}`" class="diagnostic" @click="focusSpan(diagnostic.start, diagnostic.end)">
          <span class="warn">!</span>
          <span><b>{{ diagnostic.code }}</b><small>{{ diagnostic.message }}</small></span>
          <code>{{ diagnostic.start }}..{{ diagnostic.end }}</code>
        </button>
      </article>
    </section>

    <footer>Spans use half-open UTF-8 byte ranges · Results come from the Rust crate</footer>
  </main>
</template>
