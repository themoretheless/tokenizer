<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { LANGUAGES, defaultModeFor, sampleFor } from './languages.js'
import { runTokenizer } from './tokenizer.js'

const source = ref(sampleFor('json'))
const language = ref('json')
const mode = ref('strict')
const layer = ref('semantic')
const result = ref(null)
const loading = ref(false)
const error = ref('')
const activeToken = ref(null)
let requestId = 0
let timer

const currentMeta = computed(() => LANGUAGES.find((item) => item.id === language.value))
const modes = computed(() => currentMeta.value?.modes ?? ['default'])
const showModePicker = computed(() => modes.value.length > 1 || language.value === 'json')
const visibleTokens = computed(() => result.value?.tokens ?? [])
const diagnostics = computed(() => result.value?.diagnostics ?? [])
const active = computed(() => visibleTokens.value.find((token) => token.index === activeToken.value))
const effectiveMode = computed(() => {
  if (language.value === 'json') return mode.value === 'jsonc' ? 'jsonc' : 'strict'
  return modes.value.includes(mode.value) ? mode.value : defaultModeFor(language.value)
})

async function tokenize() {
  const id = ++requestId
  loading.value = true
  error.value = ''
  try {
    const payload = await runTokenizer({
      source: source.value,
      language: language.value,
      mode: effectiveMode.value,
      layer: layer.value,
    })
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

function onLanguageChange(event) {
  const next = event.target.value
  language.value = next
  mode.value = defaultModeFor(next)
  source.value = sampleFor(next)
}

function loadSample() {
  source.value = sampleFor(language.value)
}

function loadJsonRecovery() {
  language.value = 'json'
  mode.value = 'strict'
  source.value = `{
  "ready": true,
  "leadingZero": 01,
  "broken": "escape\\q",
  "missing":
}`
}

function focusSpan(start, end) {
  const token = visibleTokens.value.find((item) => item.start < Math.max(end, start + 1) && item.end > start)
  activeToken.value = token?.index ?? null
}

watch([source, language, mode, layer], () => {
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
      <label class="lang-select">
        <span>Language</span>
        <select :value="language" aria-label="Language" @change="onLanguageChange">
          <option v-for="item in LANGUAGES" :key="item.id" :value="item.id">
            {{ item.label }}
          </option>
        </select>
      </label>

      <div v-if="showModePicker" class="segmented" aria-label="Dialect mode">
        <button
          v-for="item in modes"
          :key="item"
          :class="{ selected: effectiveMode === item }"
          @click="mode = item"
        >
          {{ item === 'strict' ? 'Strict JSON' : item === 'jsonc' ? 'JSONC' : item }}
        </button>
      </div>

      <div class="segmented" aria-label="Token layer">
        <button :class="{ selected: layer === 'semantic' }" @click="layer = 'semantic'">Semantic</button>
        <button :class="{ selected: layer === 'syntax' }" @click="layer = 'syntax'">Syntax</button>
      </div>

      <div class="examples">
        <span>Load</span>
        <button type="button" @click="loadSample">Sample</button>
        <button v-if="language === 'json'" type="button" @click="loadJsonRecovery">Recovery</button>
      </div>
    </section>

    <section class="workspace">
      <article class="panel editor-panel">
        <div class="panel-head">
          <span>01 / INPUT</span>
          <span>{{ language }} · {{ result?.sourceBytes ?? 0 }} UTF-8 BYTES</span>
        </div>
        <textarea v-model="source" spellcheck="false" :aria-label="`${language} source`" />
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
        <div v-if="active" class="inspector">
          <b>{{ active.kind }}</b>
          <code>{{ active.start }}..{{ active.end }}</code>
          <span>{{ active.end - active.start }} bytes</span>
        </div>
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
              <tr
                v-for="token in visibleTokens"
                :key="token.index"
                :class="{ active: activeToken === token.index }"
                @mouseenter="activeToken = token.index"
                @mouseleave="activeToken = null"
              >
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
        <div v-if="!diagnostics.length" class="all-clear">
          <span>✓</span>
          <b>No diagnostics</b>
          <small>The input is valid in this mode.</small>
        </div>
        <button
          v-for="diagnostic in diagnostics"
          :key="`${diagnostic.code}-${diagnostic.start}`"
          class="diagnostic"
          @click="focusSpan(diagnostic.start, diagnostic.end)"
        >
          <span class="warn">!</span>
          <span>
            <b>{{ diagnostic.code }}</b>
            <small>{{ diagnostic.message }}</small>
          </span>
          <code>{{ diagnostic.start }}..{{ diagnostic.end }}</code>
        </button>
      </article>
    </section>

    <footer>
      {{ LANGUAGES.length }} languages · Spans use half-open UTF-8 byte ranges · Results come from the Rust crate
    </footer>
  </main>
</template>
