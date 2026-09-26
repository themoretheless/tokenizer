<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { LANGUAGES, defaultModeFor, groupLabelFor, groupedLanguages, languageMeta, modeLabelFor, sampleFor } from './languages.js'
import { INVISIBLE_KINDS, kindColor, kindCounts } from './kinds.js'
import { LANGUAGE_CASES } from '../tests/language-cases.js'
import { runTokenizer } from './tokenizer.js'
import { PRESETS, depthLabel, depthOf, loadCatalog } from './catalog.js'

const MAX_RENDERED_TOKENS = 4000
const initialParams = new URLSearchParams(window.location.search)
const initialLang = languageMeta(initialParams.get('lang')) ? initialParams.get('lang') : 'json'
const initialMode = languageMeta(initialLang).modes.includes(initialParams.get('mode'))
  ? initialParams.get('mode')
  : defaultModeFor(initialLang)
const initialLayer = initialParams.get('layer') === 'syntax' ? 'syntax' : 'semantic'
const VIEWS = [{ id: 'single', label: 'Single' }, { id: 'compare', label: 'Compare' }, { id: 'matrix', label: 'Matrix' }]
const initialView = VIEWS.some((item) => item.id === initialParams.get('view')) ? initialParams.get('view') : 'single'
const initialPreset = PRESETS.some((item) => item.id === initialParams.get('preset')) ? initialParams.get('preset') : ''
const initialSecond = languageMeta(initialParams.get('b')) ? initialParams.get('b') : 'yaml'

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
const engines = ref({})
const catalogError = ref('')
const view = ref(initialView)
const preset = ref(initialPreset)
const secondId = ref(initialSecond)
const resultB = ref(null)
const batchRows = ref([])
const batchProgress = ref({ done: 0, total: 0 })
const batchRunning = ref(false)
const DEPTH_FILTERS = [
  { id: 'any', label: 'Any depth' },
  { id: 'specific', label: 'Format-specific' },
  { id: 'generic', label: 'Generic only' },
]
const initialDepth = DEPTH_FILTERS.some((item) => item.id === initialParams.get('depth'))
  ? initialParams.get('depth')
  : 'any'
const depthFilter = ref(initialDepth)
const sortKey = ref('id')
const sortDir = ref(1)
let requestId = 0
let requestIdB = 0
let batchToken = 0
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
const currentEngine = computed(() => engines.value[language.value])
const secondEngine = computed(() => engines.value[secondId.value])

/// Whether an engine issues a validity verdict at all. `validate` is advertised
/// only by grammars that reject their own invalid input; the shared fullkit
/// parser behind the wave languages flags valid code and stays quiet on broken
/// code, so for those engines a diagnostic count reports what recovery managed
/// to notice, not whether the document is well-formed.
function engineValidates(id) {
  return engines.value[id]?.capabilities?.includes('validate') ?? false
}
const currentValidates = computed(() => engineValidates(language.value))
const secondValidates = computed(() => engineValidates(secondId.value))
const secondModes = computed(() => languageMeta(secondId.value)?.modes ?? ['default'])
const secondMeta = computed(() => languageMeta(secondId.value))
const familyLabel = computed(() => (currentEngine.value ? currentEngine.value.family : null))
const presetCounts = computed(() => {
  const counts = { '': Object.keys(engines.value).length }
  for (const item of PRESETS) counts[item.id] = presetIds(item.id).length
  return counts
})
const visibleLegendB = computed(() => kindCounts(resultB.value?.tokens ?? []))
const depthB = computed(() => (resultB.value ? depthOf(resultB.value.tokens) : null))
const depthA = computed(() => (result.value ? depthOf(result.value.tokens) : null))
const depthFilterCounts = computed(() => {
  const counts = { any: batchRows.value.length, specific: 0, generic: 0 }
  for (const row of batchRows.value) counts[row.specific ? 'specific' : 'generic'] += 1
  return counts
})
const matrixRows = computed(() => {
  const rows = batchRows.value.filter((row) => {
    if (depthFilter.value === 'specific') return row.specific > 0
    if (depthFilter.value === 'generic') return !row.specific
    return true
  })
  const key = sortKey.value
  const dir = sortDir.value
  return rows.sort((a, b) => {
    const av = a[key]
    const bv = b[key]
    if (typeof av === 'string') return dir * av.localeCompare(bv)
    return dir * (av - bv)
  })
})
const compareDiff = computed(() => {
  if (!result.value || !resultB.value) return null
  const a = new Set(result.value.tokens.map((token) => token.kind))
  const b = new Set(resultB.value.tokens.map((token) => token.kind))
  return {
    onlyA: [...a].filter((kind) => !b.has(kind)).sort(),
    onlyB: [...b].filter((kind) => !a.has(kind)).sort(),
    shared: [...a].filter((kind) => b.has(kind)).sort(),
  }
})

const compareOptions = computed(() => LANGUAGES.filter((item) => item.id !== language.value))

function pickFromMatrix(id) {
  if (!languageMeta(id)) return
  view.value = 'single'
  batchRunning.value = false
  setLanguage(id)
}

function presetIds(presetId) {
  if (!presetId) return Object.keys(engines.value)
  return Object.keys(engines.value).filter((id) => engines.value[id].presets.includes(presetId))
}

function filteredGroupsInner() {
  const groups = groupedLanguages(comboSearch.value)
  if (!preset.value || !Object.keys(engines.value).length) return groups
  const allowed = new Set(presetIds(preset.value))
  return groups
    .map((group) => ({ ...group, languages: group.languages.filter((item) => allowed.has(item.id)) }))
    .filter((group) => group.languages.length)
}

const filteredGroups = computed(filteredGroupsInner)
const flatMatches = computed(() => filteredGroups.value.flatMap((group) => group.languages))
const statusLabel = computed(() => {
  if (loading.value) return 'ANALYZING'
  if (error.value) return 'BRIDGE ERROR'
  if (!result.value) return 'READY'
  const count = diagnostics.value.length
  if (!currentValidates.value) {
    return count ? `${count} FLAG${count === 1 ? '' : 'S'} · NOT VALIDATED` : 'TOKENIZED'
  }
  if (result.value.valid) return 'VALID'
  return `${diagnostics.value.length} DIAG${diagnostics.value.length === 1 ? '' : 'S'}`
})
const statusBad = computed(() =>
  Boolean(error.value) || Boolean(result.value && currentValidates.value && !result.value.valid),
)

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

async function tokenizeSecond() {
  if (view.value !== 'compare') { resultB.value = null; return }
  const id = ++requestIdB
  try {
    const payload = await runTokenizer({
      source: source.value,
      language: secondId.value,
      mode: defaultModeFor(secondId.value),
      layer: layer.value,
    })
    if (id === requestIdB) { resultB.value = payload; error.value = '' }
  } catch (cause) {
    if (id === requestIdB) { resultB.value = null; error.value = cause.message }
  }
}

function setSecondLanguage(next) {
  if (!languageMeta(next)) return
  secondId.value = next
  resultB.value = null
  tokenizeSecond()
}

function setPreset(next) {
  preset.value = next
  comboOpen.value = true
}

function setDepthFilter(next) {
  depthFilter.value = next
}

function sortBy(key) {
  if (sortKey.value === key) sortDir.value = -sortDir.value
  else {
    sortKey.value = key
    sortDir.value = key === 'id' || key === 'family' || key === 'depth' ? 1 : -1
  }
}

function setView(next) {
  view.value = next
  if (next !== 'compare') resultB.value = null
  if (next === 'compare') tokenizeSecond()
  if (next !== 'matrix') batchRunning.value = false
}

async function runBatch() {
  const ids = presetIds(preset.value)
  const token = ++batchToken
  batchRunning.value = true
  batchRows.value = []
  batchProgress.value = { done: 0, total: ids.length }
  for (const id of ids.sort((a, b) => a.localeCompare(b))) {
    const started = performance.now()
    let row
    try {
      const payload = await runTokenizer({
        source: sampleFor(id),
        language: id,
        mode: defaultModeFor(id),
        layer: layer.value,
      })
      const depth = depthOf(payload.tokens)
      row = {
        id,
        family: engines.value[id]?.family ?? '?',
        depth: depthLabel(engines.value[id]),
        presets: engines.value[id]?.presets ?? [],
        tokens: payload.tokens.length,
        kinds: depth.kinds,
        specific: depth.specific,
        diagnostics: payload.diagnostics.length,
        valid: payload.valid,
        ms: Math.round(performance.now() - started),
        note: '',
      }
    } catch (cause) {
      row = { id, family: '?', depth: '?', presets: [], tokens: 0, kinds: 0, specific: 0, diagnostics: 0, valid: false, ms: 0, note: cause.message }
    }
    if (token !== batchToken) return
    batchRows.value = [...batchRows.value, row]
    batchProgress.value = { done: batchRows.value.length, total: ids.length }
  }
  if (token === batchToken) batchRunning.value = false
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
  timer = setTimeout(() => {
    tokenize()
    tokenizeSecond()
  }, 180)
}, { immediate: true })

watch([view, preset], () => {
  if (view.value === 'matrix') runBatch()
})

watch(preset, () => {
  if (view.value === 'matrix') runBatch()
})

watch([language, mode, layer, view, preset, secondId, depthFilter], () => {
  clearTimeout(urlTimer)
  urlTimer = setTimeout(() => {
    const params = new URLSearchParams(window.location.search)
    params.set('lang', language.value)
    params.set('mode', effectiveMode.value)
    params.set('layer', layer.value)
    params.set('view', view.value)
    params.set('preset', preset.value)
    params.set('depth', depthFilter.value)
    if (view.value === 'compare') params.set('b', secondId.value)
    window.history.replaceState(null, '', `${window.location.pathname}?${params}`)
  }, 250)
})

onMounted(async () => {
  document.addEventListener('click', onDocumentClick)
  try {
    engines.value = await loadCatalog()
    if (view.value === 'matrix') runBatch()
  } catch (cause) {
    catalogError.value = cause.message
  }
})
onBeforeUnmount(() => {
  batchToken += 1
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
      <div class="status" :class="{ bad: statusBad }" aria-live="polite">
        <span class="pulse" />
        {{ statusLabel }}
      </div>
    </header>

    <section class="controls">
      <div class="combo field-label">
        <span>
          {{ familyLabel === 'format' ? 'Format' : 'Language' }} ·
          {{ currentMeta && groupLabelFor(currentMeta.group) }} ·
          {{ currentEngine ? depthLabel(currentEngine) : 'loading registry…' }}
        </span>
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
                <em v-if="engines[item.id]?.family === 'format'" class="fam-badge">format</em>
              </button>
            </template>
            <p v-if="!flatMatches.length" class="combo-empty">No language matches.</p>
          </div>
        </div>
      </div>

      <div class="segmented" aria-label="Engine set">
        <button :class="{ selected: preset === '' }" @click="setPreset('')">
          All<small>{{ presetCounts[''] || '…' }}</small>
        </button>
        <button
          v-for="item in PRESETS"
          :key="item.id"
          :class="{ selected: preset === item.id }"
          @click="setPreset(item.id)"
        >
          {{ item.label }}<small>{{ presetCounts[item.id] || '…' }}</small>
        </button>
      </div>

      <div class="segmented" aria-label="View">
        <button
          v-for="item in VIEWS"
          :key="item.id"
          :class="{ selected: view === item.id }"
          @click="setView(item.id)"
        >
          {{ item.label }}
        </button>
      </div>

      <div v-if="view === 'compare'" class="combo field-label">
        <span>Against · {{ secondMeta && groupLabelFor(secondMeta.group) }}</span>
        <select :value="secondId" aria-label="Compare engine" @change="setSecondLanguage($event.target.value)">
          <option v-for="item in compareOptions" :key="item.id" :value="item.id" :disabled="item.id === language">
            {{ item.label }} · {{ item.id }}
          </option>
        </select>
      </div>

      <div v-if="showModePicker && view !== 'matrix'" class="segmented" aria-label="Dialect mode">
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

    <p v-if="catalogError" class="bridge-error">Registry catalog unavailable: {{ catalogError }}</p>

    <section v-if="view === 'matrix'" class="panel matrix-panel">
      <div class="panel-head">
        <span>02 / MATRIX · {{ preset || 'all engines' }}</span>
        <span>{{ batchProgress.done }}/{{ batchProgress.total }} · layer {{ layer }}</span>
      </div>
      <div class="matrix-controls">
        <div class="segmented" aria-label="Depth filter">
          <button
            v-for="item in DEPTH_FILTERS"
            :key="item.id"
            :class="{ selected: depthFilter === item.id }"
            type="button"
            @click="setDepthFilter(item.id)"
          >
            {{ item.label }}<small> {{ depthFilterCounts[item.id] }}</small>
          </button>
        </div>
        <button type="button" class="sort-reset" :disabled="sortKey === 'id' && sortDir === 1" @click="sortBy('id')">
          Reset sort
        </button>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th><button type="button" class="th-sort" @click="sortBy('id')">Engine<span v-if="sortKey === 'id'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('family')">Family<span v-if="sortKey === 'family'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('depth')">Depth<span v-if="sortKey === 'depth'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('tokens')">Tokens<span v-if="sortKey === 'tokens'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('kinds')">Kinds<span v-if="sortKey === 'kinds'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('specific')">Format-specific<span v-if="sortKey === 'specific'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('diagnostics')">Diags<span v-if="sortKey === 'diagnostics'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
              <th><button type="button" class="th-sort" @click="sortBy('ms')">ms<span v-if="sortKey === 'ms'">{{ sortDir > 0 ? ' ▲' : ' ▼' }}</span></button></th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in matrixRows"
              :key="row.id"
              :class="{
                invalid: !row.valid && !row.note && engineValidates(row.id),
                unvalidated: !row.valid && !row.note && !engineValidates(row.id),
              }"
            >
              <td><button type="button" class="row-link" @click="pickFromMatrix(row.id)">{{ row.id }}</button></td>
              <td>{{ row.family }}</td>
              <td><code>{{ row.depth }}</code></td>
              <td>{{ row.tokens }}</td>
              <td>{{ row.kinds }}</td>
              <td>
                <b v-if="!row.specific" class="warn-flag">none</b>
                <template v-else>{{ row.specific }}</template>
              </td>
              <td>{{ row.diagnostics }}</td>
              <td><small>{{ row.note || row.ms }}</small></td>
            </tr>
            <tr v-if="!matrixRows.length && !batchRunning">
              <td colspan="8"><small class="matrix-empty">No engines match this depth filter.</small></td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="matrix-note">
        “Format-specific” counts emitted token kinds outside the generic
        identifier/keyword/number/string/punctuation vocabulary — an engine that
        shows <b>none</b> has no format-aware lexing yet.
      </p>
    </section>

    <section v-else class="workspace" :class="{ 'is-compare': view === 'compare' }">
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
        <div class="panel-head">
          <span>{{ language }} · {{ familyLabel === 'format' ? 'format' : 'language' }}</span>
          <span>{{ visibleTokens.length }} TOKENS<span v-if="depthA"> · {{ depthA.kinds }} KINDS · {{ depthA.specific }} SPECIFIC</span></span>
        </div>
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

      <article v-if="view === 'compare'" class="panel output-panel">
        <div class="panel-head">
          <span>{{ secondId }} · {{ secondEngine?.family === 'format' ? 'format' : 'language' }}</span>
          <span>{{ resultB?.tokens.length ?? 0 }} TOKENS<span v-if="depthB"> · {{ depthB.kinds }} KINDS · {{ depthB.specific }} SPECIFIC</span></span>
        </div>
        <div v-if="visibleLegendB.length" class="legend" aria-label="Token kinds (right)">
          <button
            v-for="item in visibleLegendB"
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
          v-for="token in resultB?.tokens ?? []"
          :key="token.index"
          :class="['token', { ws: INVISIBLE_KINDS.has(token.kind), dim: kindFilter !== null && token.kind !== kindFilter }]"
          :style="tokenStyle(token)"
          :title="`${token.kind} · ${token.start}..${token.end}`"
        >{{ token.text }}</span><span v-if="!resultB" class="empty">Tokenizing…</span></pre>
        <div class="compare-verdict">
          <span>{{ secondModes.join('/') }} · {{ secondValidates
            ? (resultB?.valid ? 'valid' : (resultB?.diagnostics.length ?? 0) + ' diagnostics')
            : (resultB?.diagnostics.length ?? 0) + ' flags, not validated' }}</span>
        </div>
      </article>
    </section>

    <section v-if="view === 'compare' && compareDiff" class="panel diff-panel">
      <div class="panel-head"><span>KIND DIFF</span><span>{{ language }} ⇄ {{ secondId }}</span></div>
      <div class="diff-cols">
        <p><b>only {{ language }}</b><span v-if="!compareDiff.onlyA.length" class="empty">none</span><code v-for="kind in compareDiff.onlyA" :key="kind" :style="{ color: dotColor(kind) }">{{ kind }}</code></p>
        <p><b>shared</b><code v-for="kind in compareDiff.shared" :key="kind" :style="{ color: dotColor(kind) }">{{ kind }}</code></p>
        <p><b>only {{ secondId }}</b><span v-if="!compareDiff.onlyB.length" class="empty">none</span><code v-for="kind in compareDiff.onlyB" :key="kind" :style="{ color: dotColor(kind) }">{{ kind }}</code></p>
      </div>
    </section>

    <p v-if="error" class="bridge-error">{{ error }}</p>

    <section v-if="view === 'single'" class="lower-grid">
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
          <small v-if="currentValidates">The input is valid in this mode.</small>
          <small v-else>This engine does not advertise validation, so nothing was checked.</small>
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
      {{ Object.keys(engines).length }} engines · {{ PRESETS.map((item) => `${item.label} ${presetCounts[item.id] ?? '…'}`).join(' · ') }} · Spans use half-open UTF-8 byte ranges · Results come from the Rust crate
    </footer>
  </main>
</template>
