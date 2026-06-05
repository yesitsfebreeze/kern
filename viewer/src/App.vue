<script setup>
import { ref, onMounted, onBeforeUnmount, nextTick } from 'vue'

const KIND = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }
const MARK = { Fact: '◆', Document: '■', Question: '▲', Claim: '●' }

const stats = ref('')
const err = ref('')
const turns = ref([])      // {role:'user'|'oracle', text, sources?, chains?}
const sources = ref([])    // current answer's source tiles
const chains = ref([])     // current answer's provenance strings
const input = ref('')
const busy = ref(false)
const hot = ref(null)      // hovered/active citation number
const inputEl = ref(null)
const scrollEl = ref(null)

let history = []           // [{role, content}] sent to the server
let ctrl = null            // AbortController for the in-flight stream
let pulse = null

const heatMax = ref(1)
function ramp(h) {
  const t = Math.min(1, Math.sqrt((h || 0) / (heatMax.value || 1)))
  const lo = [42, 24, 9], hi = [255, 226, 166]
  const c = lo.map((v, i) => Math.round(v + (hi[i] - v) * (0.12 + 0.85 * t)))
  return `rgb(${c[0]},${c[1]},${c[2]})`
}
function textColor(bg) {
  const m = bg.match(/\d+/g) || [0, 0, 0]
  return (0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2]) / 255 > 0.62 ? '#1c1206' : '#fdfaf3'
}

// Split an answer into text + [n] citation chips for rendering.
function segments(text) {
  const out = []
  const re = /\[(\d+)\]/g
  let last = 0, m
  while ((m = re.exec(text))) {
    if (m.index > last) out.push({ t: text.slice(last, m.index) })
    out.push({ cite: +m[1] })
    last = m.index + m[0].length
  }
  if (last < text.length) out.push({ t: text.slice(last) })
  return out
}

async function loadStats() {
  try {
    const g = await (await fetch('/graph')).json()
    const groups = (g.kerns || []).filter(k => k.id !== '__all__').length
    heatMax.value = Math.max(1, ...(g.nodes || []).map(n => +n.heat || 0))
    stats.value = `${(g.nodes || []).length} thoughts · ${groups} groups`
    err.value = ''
  } catch (e) { err.value = String(e) }
}

async function ask() {
  const q = input.value.trim()
  if (!q || busy.value) return
  if (ctrl) ctrl.abort()
  ctrl = new AbortController()
  input.value = ''
  busy.value = true
  sources.value = []; chains.value = []; hot.value = null
  turns.value.push({ role: 'user', text: q })
  turns.value.push({ role: 'oracle', text: '', sources: [], chains: [] })
  const oracle = turns.value[turns.value.length - 1]
  await scrollDown()
  try {
    const res = await fetch('/ask', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ question: q, history: history.slice(-6) }),
      signal: ctrl.signal,
    })
    if (!res.ok || !res.body) throw new Error('oracle unavailable')
    const reader = res.body.getReader()
    const dec = new TextDecoder()
    let buf = ''
    for (;;) {
      const { value, done } = await reader.read()
      if (done) break
      buf += dec.decode(value, { stream: true })
      let i
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const frame = buf.slice(0, i); buf = buf.slice(i + 2)
        handleFrame(frame, oracle)
      }
    }
    history.push({ role: 'user', content: q })
    history.push({ role: 'assistant', content: oracle.text })
  } catch (e) {
    if (e.name !== 'AbortError') oracle.text = oracle.text || '⚠ oracle unavailable'
  } finally {
    busy.value = false
    await scrollDown()
  }
}

// Parse one SSE frame ("event: x\ndata: {json}") and apply it.
function handleFrame(frame, oracle) {
  let ev = 'message', data = ''
  for (const line of frame.split('\n')) {
    if (line.startsWith('event:')) ev = line.slice(6).trim()
    else if (line.startsWith('data:')) data += (data ? '\n' : '') + line.slice(5).trim()
  }
  let d = {}
  try { d = data ? JSON.parse(data) : {} } catch (_) { return }
  if (ev === 'sources') {
    sources.value = d.entities || []
    chains.value = d.chains || []
    oracle.sources = sources.value; oracle.chains = chains.value
    scrollDown()
  } else if (ev === 'token') {
    oracle.text += d.t || ''
    scrollDown()
  } else if (ev === 'error') {
    oracle.text = (oracle.text || '') + `\n⚠ ${d.message || 'oracle error'}`
  }
}

async function scrollDown() {
  await nextTick()
  const el = scrollEl.value
  if (el) el.scrollTop = el.scrollHeight
}

function onKey(ev) {
  if (ev.key === 'Enter' && !ev.shiftKey) { ev.preventDefault(); ask() }
}

onMounted(() => {
  loadStats(); pulse = setInterval(loadStats, 5000)
  inputEl.value?.focus()
})
onBeforeUnmount(() => { if (pulse) clearInterval(pulse); if (ctrl) ctrl.abort() })
</script>

<template>
  <div class="app">
    <header class="rail">
      <div class="brand"><b>kern</b><span class="sub">oracle</span></div>
      <div class="rstats"><span class="dot"></span>{{ stats }}<span v-if="err" class="err"> — {{ err }}</span></div>
    </header>

    <div class="stage">
      <section class="chat">
        <div class="scroll" ref="scrollEl">
          <div v-if="!turns.length" class="hint">Ask the oracle anything about your memory.</div>
          <div v-for="(t, i) in turns" :key="i" class="turn" :class="t.role">
            <template v-if="t.role === 'oracle'">
              <span class="oglyph">◈</span>
              <div class="obody">
                <span v-for="(s, j) in segments(t.text)" :key="j">
                  <span v-if="s.cite" class="cite" :class="{ on: hot === s.cite }"
                    @mouseenter="hot = s.cite" @mouseleave="hot = null">{{ s.cite }}</span>
                  <span v-else>{{ s.t }}</span>
                </span>
                <span v-if="busy && i === turns.length - 1" class="caret">▍</span>
              </div>
            </template>
            <div v-else class="ubody">{{ t.text }}</div>
          </div>
        </div>
        <div class="ask">
          <input ref="inputEl" v-model="input" @keydown="onKey" :disabled="busy"
            placeholder="ask the oracle…" />
          <button @click="ask" :disabled="busy || !input.trim()">↵</button>
        </div>
      </section>

      <section class="panel">
        <header class="phead"><span class="ptitle">sources</span>
          <span class="count" v-if="sources.length">{{ sources.length }}</span></header>
        <div class="bwrap">
          <div class="bento">
            <div v-for="s in sources" :key="s.id" class="tile" :class="{ on: hot === s.n }"
              :style="{ background: ramp(s.heat), color: textColor(ramp(s.heat)) }"
              @mouseenter="hot = s.n" @mouseleave="hot = null">
              <span class="tn">{{ s.n }}</span>
              <span class="tmark">{{ MARK[s.kind] || '·' }}</span>
              <div class="tname">{{ s.label }}</div>
              <div class="tmeta">{{ s.kind }} · {{ (+s.score).toFixed(2) }}</div>
            </div>
            <div v-if="!sources.length" class="empty">sources appear here</div>
          </div>
          <pre v-if="chains.length" class="trail">{{ chains.join('\n') }}</pre>
        </div>
      </section>
    </div>
  </div>
</template>

<style>
:root { --ink:#f4f1ea; --muted:#8b8678; --line:rgba(244,241,234,0.10); --panel:rgba(244,241,234,0.018);
  --display:'Bricolage Grotesque',system-ui,sans-serif; --body:'Hanken Grotesk',system-ui,sans-serif; --mono:'IBM Plex Mono',ui-monospace,monospace; }
* { box-sizing:border-box; } html,body,#app { height:100%; margin:0; }
.app { height:100%; display:flex; flex-direction:column;
  background:radial-gradient(120% 90% at 50% -10%, #16130f 0%, #0a0a0c 55%, #08080a 100%); color:var(--ink); font-family:var(--body); }
.rail { display:flex; align-items:baseline; gap:14px; padding:14px 22px; border-bottom:1px solid var(--line); }
.brand b { font-family:var(--display); font-weight:800; font-size:17px; } .brand .sub { color:var(--muted); font-family:var(--mono); font-size:11px; letter-spacing:.16em; text-transform:uppercase; margin-left:6px; }
.rstats { color:var(--muted); font-family:var(--mono); font-size:11px; display:flex; align-items:center; gap:8px; }
.dot { width:7px; height:7px; border-radius:50%; background:#98c379; box-shadow:0 0 8px #98c379; }
.err { color:#e8705e; }
.stage { flex:1; min-height:0; display:flex; gap:18px; padding:18px 22px; }
.chat { flex:1; min-width:0; display:flex; flex-direction:column; border-radius:18px; background:var(--panel); box-shadow:inset 0 0 0 1px rgba(244,241,234,0.06); overflow:hidden; }
.scroll { flex:1; overflow-y:auto; padding:20px; display:flex; flex-direction:column; gap:16px; }
.hint { color:var(--muted); margin:auto; font-size:14px; }
.turn.user { align-self:flex-end; max-width:80%; } .ubody { background:rgba(242,169,62,0.14); color:var(--ink); padding:9px 13px; border-radius:13px 13px 3px 13px; font-size:14px; }
.turn.oracle { display:flex; gap:10px; max-width:92%; } .oglyph { color:#98c379; } .obody { font-size:15px; line-height:1.5; }
.cite { display:inline-flex; align-items:center; justify-content:center; min-width:17px; height:17px; padding:0 4px; margin:0 1px; border-radius:5px;
  background:rgba(97,175,239,0.18); color:#9cd0ff; font-family:var(--mono); font-size:11px; cursor:pointer; vertical-align:1px; }
.cite.on { background:#61afef; color:#08080a; }
.caret { color:#f2a93e; animation:blink 1s steps(2) infinite; } @keyframes blink { 50% { opacity:0; } }
.ask { display:flex; gap:10px; padding:14px; border-top:1px solid var(--line); }
.ask input { flex:1; background:#131210; border:0; outline:none; color:var(--ink); font-family:var(--body); font-size:14px; padding:13px 15px; border-radius:11px; box-shadow:inset 0 0 0 1px var(--line); }
.ask input::placeholder { color:#4f4b43; } .ask button { background:#f2a93e; color:#1c1206; border:0; border-radius:11px; width:46px; font-size:16px; cursor:pointer; } .ask button:disabled { opacity:.4; cursor:default; }
.panel { width:40%; min-width:300px; display:flex; flex-direction:column; border-radius:18px; background:var(--panel); box-shadow:inset 0 0 0 1px rgba(244,241,234,0.06); overflow:hidden; }
.phead { font-family:var(--mono); font-size:11px; letter-spacing:.16em; text-transform:uppercase; padding:13px 16px; border-bottom:1px solid var(--line); display:flex; gap:10px; }
.phead .count { margin-left:auto; color:var(--muted); }
.bwrap { flex:1; overflow-y:auto; padding:14px; }
.bento { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
.tile { position:relative; border-radius:13px; padding:12px; min-height:96px; display:flex; flex-direction:column; justify-content:flex-end; gap:5px; box-shadow:inset 0 0 0 1px rgba(255,255,255,0.08); transition:box-shadow .12s; }
.tile.on { box-shadow:inset 0 0 0 2px #61afef, 0 0 0 2px rgba(97,175,239,0.4); }
.tn { position:absolute; top:9px; left:11px; font-family:var(--mono); font-size:11px; opacity:.8; } .tmark { position:absolute; top:8px; right:11px; opacity:.8; }
.tname { font-family:var(--display); font-weight:800; font-size:14px; line-height:1.12; display:-webkit-box; -webkit-line-clamp:3; -webkit-box-orient:vertical; overflow:hidden; }
.tmeta { font-family:var(--mono); font-size:10px; opacity:.75; }
.empty { grid-column:1/-1; color:var(--muted); text-align:center; padding:30px 0; font-size:13px; }
.trail { margin-top:14px; padding:12px; border-radius:11px; background:#0d0c0b; box-shadow:inset 0 0 0 1px var(--line);
  font-family:var(--mono); font-size:11px; line-height:1.5; color:var(--muted); white-space:pre-wrap; }
</style>
