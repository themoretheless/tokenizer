<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { defaultModeFor, groupLabelFor, groupedLanguages, languageMeta, modeLabelFor, sampleFor } from './languages.js'
import { INVISIBLE_KINDS, kindColor, kindCounts } from './kinds.js'
import { LANGUAGE_CASES } from '../tests/language-cases.js'
import { runTokenizer } from './tokenizer.js'

const MAX_RENDERED_TOKENS = 4000
const initialParams = new URLSearchParams(window.location.search)
const initialLang = languageMeta(initialParams.get('lang')) ? initialParams.get('lang') : 'json'
const initialMode = languageMeta(initialLang).modes.includes(initialParams.get('mode'))
  ? initialParams.get('mode')
  : defaultModeFor(initialLang)
const initialLayer = initialParams.get('layer') === 'syntax' ? 'syntax' : 'semantic'

const source = ref(sampleFor(initialLang))
const language = ref(initialLang)
const mode = ref(initialMode)
const layer = ref(initialLayer)
const result = ref(null)
const loading = ref(false)
const error = ref('')
const activeToken = ref(null)
const kindHover = ref(null)
const kindPin = ref(null)
const activeCase = ref('sample')
const comboOpen = ref(false)
const comboSearch = ref('')
const comboActive = ref(0)
const cursor = ref({ line: 1, column: 1, selected: 0 })
const textareaEl = ref(null)
const gutterEl = ref(null)
const comboInputEl = ref(null)
let requestId = 0
let timer
let urlTimer

const currentMeta = computed(() => languageMeta(language.value))
const modes = computed(() => currentMeta.value?.modes ?? ['default'])
const showModePicker = computed(() => modes.value.length > 1)
const visibleTokens = computed(() => result.value?.tokens ?? [])
const renderedTokens = computed(() => visibleTokens.value.slice(0, MAX_RENDERED_TOKENS))
const truncated = computed(() => visibleTokens.value.length > MAX_RENDERED_TOKENS)
const diagnostics = computed(() => result.value?.diagnostics ?? [])
const active = computed(() => visibleTokens.value.find((token) => token.index === activeToken.value))
const effectiveMode = computed(() => (
  modes.value.includes(mode.value) ? mode.value : defaultModeFor(language.value)
))
const kindFilter = computed(() => kindHover.value ?? kindPin.value)
const legend = computed(() => kindCounts(visibleTokens.value))
const casesForLanguage = computed(() => LANGUAGE_CASES[language.value]?.cases ?? [])
const lineCount = computed(() => source.value.split('\n').length)
const filteredGroups = computed(() => groupedLanguages(comboSearch.value))
const flatMatches = computed(() => filteredGroups.value.flatMap((group) => group.languages))
const statusLabel = computed(() => {
  if (loading.value) return 'ANALYZING'
  if (error.value) return 'BRIDGE ERROR'
  if (!result.value) return 'READY'
  if (result.value.valid) return 'VALID'
  return `${diagnostics.value.length} DIAG${diagnostics.value.length === 1 ? '' : 'S'}`
})

function dotColor(kind) {
  const color = kindColor(kind)
  return color === 'inherit' ? '#5a6a72' : color
}

function tokenStyle(token) {
  const color = kindColor(token.kind)
  return color === 'inherit' ? undefined : { color }
}

function tokenClass(token) {
  return ['token', {
    ws: INVISIBLE_KINDS.has(token.kind),
    active: activeToken.value === token.index,
    dim: kindFilter.value !== null && token.kind !== kindFilter.value,
  }]
}

function lineColumnAt(offset) {
  const upto = source.value.slice(0, offset)
  const lines = upto.split('\n')
  return { line: lines.length, column: lines[lines.length - 1].length + 1 }
}

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

function setLanguage(next) {
  language.value = next
  mode.value = defaultModeFor(next)
  source.value = sampleFor(next)
  activeCase.value = 'sample'
  comboOpen.value = false
  comboSearch.value = ''
}

function onCaseChange(name) {
  activeCase.value = name
  if (name === 'sample') {
    source.value = sampleFor(language.value)
    return
  }
  const item = casesForLanguage.value.find((entry) => entry.name === name)
  if (!item) return
  if (item.mode) mode.value = item.mode
  source.value = item.source
}

function toggleCombo() {
  comboOpen.value = !comboOpen.value
  if (!comboOpen.value) return
  comboActive.value = Math.max(0, flatMatches.value.findIndex((item) => item.id === language.value))
  nextTick(() => comboInputEl.value?.focus())
}

function scrollActiveOption() {
  nextTick(() => {
    document.querySelector('.combo-option.active')?.scrollIntoView({ block: 'nearest' })
  })
}

function onComboKeydown(event) {
  const list = flatMatches.value
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    comboActive.value = list.length ? (comboActive.value + 1) % list.length : 0
    scrollActiveOption()
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    comboActive.value = list.length ? (comboActive.value - 1 + list.length) % list.length : 0
    scrollActiveOption()
  } else if (event.key === 'Enter') {
    event.preventDefault()
    const picked = list[comboActive.value]
    if (picked) setLanguage(picked.id)
  } else if (event.key === 'Escape') {
    comboOpen.value = false
  }
}

function onDocumentClick(event) {
  if (comboOpen.value && !event.target.closest('.combo')) comboOpen.value = false
}

function updateCursor() {
  const el = textareaEl.value
  if (!el) return
  const start = el.selectionStart ?? 0
  const end = el.selectionEnd ?? start
  const position = lineColumnAt(start)
  cursor.value = { ...position, selected: Math.max(0, end - start) }
}

function syncGutter(event) {
  if (gutterEl.value) gutterEl.value.scrollTop = event.target.scrollTop
}

function focusSpan(start, end) {
  const token = visibleTokens.value.find((item) => item.start < Math.max(end, start + 1) && item.end > start)
  activeToken.value = token?.index ?? null
}

watch(comboSearch, () => { comboActive.value = 0 })

watch(language, () => {
  activeToken.value = null
  kindHover.value = null
  kindPin.value = null
})

watch([source, language, mode, layer], () => {
  clearTimeout(timer)
  timer = setTimeout(tokenize, 180)
}, { immediate: true })

watch([language, mode, layer], () => {
  clearTimeout(urlTimer)
  urlTimer = setTimeout(() => {
    const params = new URLSearchParams(window.location.search)
    params.set('lang', language.value)
    params.set('mode', effectiveMode.value)
    params.set('layer', layer.value)
    window.history.replaceState(null, '', `${window.location.pathname}?${params}`)
  }, 250)
})

onMounted(() => document.addEventListener('click', onDocumentClick))
onBeforeUnmount(() => {
  clearTimeout(timer)
  clearTimeout(urlTimer)
  document.removeEventListener('click', onDocumentClick)
})
</script>

<template>
  <main class="shell">
    <header class="topbar">
      <div>
        <p class="eyebrow">THEMORETHELESS / DEV TOOL</p>
        <h1>Tokenizer <i>Lab</i></h1>
      </div>
      <div class="status" :class="{ bad: error || (result && !result.valid) }" aria-live="polite">
        <span class="pulse" />
        {{ statusLabel }}
      </div>
    </header>

    <section class="controls">
      <div class="combo field-label">
        <span>Language · {{ currentMeta && groupLabelFor(currentMeta.group) }}</span>
        <button type="button" class="combo-button" :aria-expanded="comboOpen" @click="toggleCombo">
          {{ currentMeta?.label }}
          <small>{{ currentMeta?.id }}</small>
        </button>
        <div v-if="comboOpen" class="combo-pop">
          <input
            ref="comboInputEl"
            v-model="comboSearch"
            class="combo-search"
            placeholder="Search languages…"
            aria-label="Search languages"
            @keydown="onComboKeydown"
          >
          <div class="combo-list" role="listbox" aria-label="Language">
            <template v-for="group in filteredGroups" :key="group.id">
              <p class="combo-group">{{ group.label }}</p>
              <button
                v-for="item in group.languages"
                :key="item.id"
                type="button"
                role="option"
                :aria-selected="item.id === language"
                :class="['combo-option', { active: flatMatches[comboActive]?.id === item.id, selected: item.id === language }]"
                @click="setLanguage(item.id)"
                @mouseenter="comboActive = flatMatches.findIndex((entry) => entry.id === item.id)"
              >
                {{ item.label }}<small>{{ item.id }}</small>
              </button>
            </template>
            <p v-if="!flatMatches.length" class="combo-empty">No language matches.</p>
          </div>
        </div>
      </div>

      <div v-if="showModePicker" class="segmented" aria-label="Dialect mode">
        <button
          v-for="item in modes"
          :key="item"
          :class="{ selected: effectiveMode === item }"
          @click="mode = item"
        >
          {{ modeLabelFor(language, item) }}
        </button>
      </div>

      <div class="segmented" aria-label="Token layer">
        <button :class="{ selected: layer === 'semantic' }" @click="layer = 'semantic'">Semantic</button>
        <button :class="{ selected: layer === 'syntax' }" @click="layer = 'syntax'">Syntax</button>
      </div>

      <label class="case-select field-label" style="margin-left: auto">
        <span>Example</span>
        <select :value="activeCase" aria-label="Example case" @change="onCaseChange($event.target.value)">
          <option value="sample">Sample</option>
          <option v-for="item in casesForLanguage" :key="item.name" :value="item.name">
            {{ item.name }}
          </option>
        </select>
      </label>
    </section>

    <section class="workspace">
      <article class="panel editor-panel">
        <div class="panel-head">
          <span>01 / INPUT</span>
          <span>{{ language }} · L{{ cursor.line }}:{{ cursor.column }}<template v-if="cursor.selected"> · {{ cursor.selected }} SEL</template> · {{ result?.sourceBytes ?? 0 }} BYTES</span>
        </div>
        <div class="editor-wrap">
          <div ref="gutterEl" class="gutter" aria-hidden="true">
            <span v-for="n in lineCount" :key="n" :class="{ cur: n === cursor.line }">{{ n }}</span>
          </div>
          <textarea
            ref="textareaEl"
            v-model="source"
            spellcheck="false"
            :aria-label="`${language} source`"
            @scroll="syncGutter"
            @keyup="updateCursor"
            @click="updateCursor"
            @select="updateCursor"
          />
        </div>
      </article>

      <article class="panel output-panel">
        <div class="panel-head"><span>02 / TOKEN MAP</span><span>{{ visibleTokens.length }} TOKENS</span></div>
        <div v-if="legend.length" class="legend" aria-label="Token kinds">
          <button
            v-for="item in legend"
            :key="item.kind"
            type="button"
            :class="['legend-chip', { on: kindPin === item.kind }]"
            @mouseenter="kindHover = item.kind"
            @mouseleave="kindHover = null"
            @click="kindPin = kindPin === item.kind ? null : item.kind"
          >
            <span class="dot" :style="{ background: dotColor(item.kind) }" />
            {{ item.kind }} <b>{{ item.count }}</b>
          </button>
        </div>
        <pre class="highlight" aria-live="polite"><span
          v-for="token in renderedTokens"
          :key="token.index"
          :class="tokenClass(token)"
          :style="tokenStyle(token)"
          :title="`${token.kind} · ${token.start}..${token.end}`"
          @mouseenter="activeToken = token.index"
          @mouseleave="activeToken = null"
          @click="activeToken = token.index"
        >{{ token.text }}</span><span v-if="!visibleTokens.length" class="empty">Waiting for input…</span></pre>
        <p v-if="truncated" class="trunc-note">Rendering first {{ MAX_RENDERED_TOKENS }} of {{ visibleTokens.length }} tokens.</p>
        <div v-if="active" class="inspector">
          <b>{{ active.kind }}</b>
          <code>{{ active.start }}..{{ active.end }}</code>
          <span>L{{ lineColumnAt(active.start).line }}:{{ lineColumnAt(active.start).column }}</span>
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
                :class="{ active: activeToken === token.index, dim: kindFilter !== null && token.kind !== kindFilter }"
                @mouseenter="activeToken = token.index"
                @mouseleave="activeToken = null"
                @click="activeToken = token.index"
              >
                <td>{{ String(token.index + 1).padStart(2, '0') }}</td>
                <td><span class="kind-dot" :style="{ background: dotColor(token.kind) }" />{{ token.kind }}</td>
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
      {{ groupedLanguages().length }} groups · Spans use half-open UTF-8 byte ranges · Results come from the Rust crate
    </footer>
  </main>
</template>
