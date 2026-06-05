<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import * as d3 from 'd3'

// Built for reach: find anything in a few strokes.
//   · just start typing      → fuzzy palette across thoughts · groups · reasons
//   · ↑↓ select, ↵ land      → anchors it AND jumps structure to its group
//   · 1–4                    → walk straight into a reason (one key per hop)
//   · /  focus   Esc  up/clear
// The hero is the thought you're on; two calm panels orbit it — STRUCTURE (the
// kern tree) and REASONS (its reason edges). One subject, two views.

const err = ref('')
const detail = ref('')
const fresh = ref(false)
const omActive = ref(false)

const anchor = ref(null)
const searchQ = ref('')
const results = ref([])
const sel = ref(0)
const searchEl = ref(null)

const nThoughts = ref(0), nGroups = ref(0), nDaemons = ref(1)
const sphereSlots = ref([]), sphereTotal = ref(0), sphereCrumbs = ref([]), sphereKey = ref('')
const reasonSlots = ref([]), reasonTotal = ref(0), reasonKey = ref(''), reasonCount = ref(0)

let timer = null, freshT = null
let searchTimer = null
let searchSeq = 0
let raw = { nodes: [], links: [], kerns: [] }
let kernsById = {}, entsByKern = new Map(), meanHeat = {}
let nodeById = {}, adj = new Map()
let treeData = null
let sphereStack = [], anchorHist = []
let lastTopo = ''
let anchorId = ''
let lastWheel = 0

const SLOTS = ['s1', 's2', 's3', 's4']
const KIND = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }
const MARK = { Fact: '◆', Document: '■', Question: '▲', Claim: '●' }
const RKIND = {
  Supersedes: { g: '↟', c: '#e06c75' }, Ratification: { g: '✓', c: '#98c379' },
  Question: { g: '?', c: '#c678dd' }, Similarity: { g: '≈', c: '#61afef' },
  Provenance: { g: '⌖', c: '#e5c07b' }, Rephrase: { g: '↺', c: '#56b6c2' },
  Spawn: { g: '✶', c: '#9aa0aa' },
}
function rk(k) { return RKIND[k] || { g: '·', c: '#9aa0aa' } }

// ---- tree (forest-aware: the aggregator merges many daemons, each its own root)
function rootIds() {
  const present = new Set(raw.kerns.map(k => k.id))
  return raw.kerns.filter(k => !k.parent || !present.has(k.parent)).map(k => k.id)
}
function makeNode(kid, seen) {
  const k = kernsById[kid]; if (!k || seen.has(kid)) return null
  seen.add(kid)
  const node = { id: kid, label: k.named ? k.label : '(unnamed)', type: 'kern', eid: null, children: [] }
  for (const c of k.children || []) { const cn = makeNode(c, seen); if (cn) node.children.push(cn) }
  for (const e of entsByKern.get(kid) || [])
    node.children.push({ id: e.id, eid: e.id, label: e.label, type: 'entity', kind: e.kind, heat: e.heat, conf: e.conf, kern: kid })
  return node
}
function buildTree() {
  const roots = rootIds(); const seen = new Set()
  if (roots.length === 1) return makeNode(roots[0], seen) || { id: 'root', label: 'root', type: 'kern', eid: null, children: [] }
  const children = []
  for (const r of roots) { const n = makeNode(r, seen); if (n) children.push(n) }
  return { id: '__all__', label: 'all memory', type: 'kern', eid: null, children, synth: true }
}
function meanHeatOf(node) { let s = 0, n = 0; const w = x => { if (x.type === 'entity') { s += +x.heat || 0; n++ } else for (const c of x.children || []) w(c) }; w(node); return n ? s / n : 0 }
function findPath(node, id, acc = []) { acc.push(node); if (node.id === id) return acc.slice(); for (const c of node.children || []) { const r = findPath(c, id, acc); if (r) return r } acc.pop(); return null }
function findById(n, id) { if (n.id === id) return n; for (const c of n.children || []) { const r = findById(c, id); if (r) return r } return null }
function sphereName(kid) { const k = kernsById[kid]; return k ? (k.named ? k.label : '(unnamed)') : '?' }
function relevance(n) { return n.type === 'entity' ? (+n.heat || 0) : (meanHeat[n.id] || 0) }

// ---- color (warm heat ramp, cold→hot) ---------------------------------------
const heatMax = () => Math.max(0.5, d3.max(raw.nodes, n => +n.heat || 0) || 1)
const WARM = d3.interpolateRgbBasis(['#2a1809', '#7c3c17', '#cf6f25', '#f2a93e', '#ffe2a6'])
function heatFrac(h) { return Math.sqrt(Math.min(1, (h || 0) / heatMax())) }
function ramp(h) { return WARM(0.12 + 0.85 * heatFrac(h)) }
function fillOf(n) { return ramp(n.type === 'entity' ? n.heat : (meanHeat[n.id] || 0)) }
function textColor(bg) { const c = d3.color(bg); if (!c) return '#fff'; return (0.299 * c.r + 0.587 * c.g + 0.114 * c.b) / 255 > 0.62 ? '#1c1206' : '#fdfaf3' }
function pct(x) { return `${Math.round(Math.max(0, Math.min(1, x)) * 100)}%` }

function tileMeta(n) {
  if (n.type === 'kern') {
    const k = kernsById[n.id]
    const cnt = k ? k.count : 0
    const groups = (n.children || []).filter(c => c.type === 'kern').length
    return `${cnt} ${cnt === 1 ? 'thought' : 'thoughts'}${groups ? ` · ${groups} ${groups === 1 ? 'group' : 'groups'}` : ''}`
  }
  return `${n.kind} · heat ${(+n.heat).toFixed(1)}`
}

// ---- LEFT: structure (the kern tree; only groups live on the stack) ----------
function relayoutSphere() {
  const cur = sphereStack[sphereStack.length - 1]; if (!cur) return
  sphereKey.value = cur.id
  sphereCrumbs.value = sphereStack.map((n, i) => ({ id: n.id, label: i === 0 ? (n.synth ? 'all memory' : 'root') : n.label }))
  const sorted = (cur.children || []).slice().sort((a, b) => relevance(b) - relevance(a))
  sphereSlots.value = sorted.slice(0, 4).map((ref, i) => ({ ref, cls: SLOTS[i] }))
  sphereTotal.value = sorted.length
}
function sphereClick(ref) {
  if (ref.type === 'kern') { sphereStack.push(ref); relayoutSphere() }
  else setAnchor(ref.eid || ref.id)
}
function sphereOut() { if (sphereStack.length > 1) { sphereStack.pop(); relayoutSphere() } }
function goCrumb(id) { const i = sphereStack.findIndex(n => n.id === id); if (i >= 0) { sphereStack.length = i + 1; relayoutSphere() } }
function gotoKern(kid) {
  if (!treeData || kid == null) return
  const path = findPath(treeData, kid); if (!path) return
  sphereStack = path.filter(n => n.type === 'kern'); if (!sphereStack.length) sphereStack = [treeData]
  relayoutSphere()
}

// ---- RIGHT: reasons (anchor's reason edges) ---------------------------------
function neighborsOf(eid) {
  const seen = new Map()
  for (const e of (adj.get(eid) || [])) { const nb = nodeById[e.id]; if (!nb) continue; if (!seen.has(e.id)) seen.set(e.id, { ...nb, eid: nb.id, type: 'entity', edge: e }) }
  return [...seen.values()].sort((a, b) => (b.heat || 0) - (a.heat || 0))
}
function relayoutReason() {
  const a = anchor.value
  if (!a) { reasonSlots.value = []; reasonTotal.value = 0; reasonKey.value = ''; reasonCount.value = 0; return }
  reasonKey.value = a.id
  const ns = neighborsOf(a.id)
  reasonSlots.value = ns.slice(0, 4).map((ref, i) => ({ ref, cls: SLOTS[i], n: i + 1 }))
  reasonTotal.value = ns.length
  reasonCount.value = (adj.get(a.id) || []).length
}
function setAnchor(eid, push = true, jump = false) {
  const n = nodeById[eid]; if (!n) return
  if (push && anchorId && anchorId !== eid) anchorHist.push(anchorId)
  anchorId = eid; anchor.value = n
  relayoutReason()
  if (jump) gotoKern(n.kern)
}
function anchorBack() { const prev = anchorHist.pop(); if (prev && nodeById[prev]) setAnchor(prev, false) }
function gotoAnchorGroup() { if (anchor.value) gotoKern(anchor.value.kern) }

function wheel(panel, ev) {
  const now = performance.now(); if (now - lastWheel < 320) return; lastWheel = now
  if (panel === 'sphere') { if (ev.deltaY < 0) { const t = sphereSlots.value[0]; if (t) sphereClick(t.ref) } else sphereOut() }
  else { if (ev.deltaY < 0) { const t = reasonSlots.value[0]; if (t) setAnchor(t.ref.eid) } else anchorBack() }
}

// ---- finder: fuzzy palette across thoughts · groups · reasons ----------------
function fuzzy(q, s) {
  s = s.toLowerCase()
  const idx = s.indexOf(q)
  if (idx >= 0) return { sc: 1000 - idx * 2 - (s.length - q.length) * 0.4, ranges: [[idx, idx + q.length]] }
  let i = 0, start = -1, last = -2; const ranges = []
  for (let j = 0; j < s.length && i < q.length; j++) {
    if (s[j] === q[i]) { if (j !== last + 1) { if (start >= 0) ranges.push([start, last + 1]); start = j } last = j; i++ }
  }
  if (i < q.length) return null
  if (start >= 0) ranges.push([start, last + 1])
  return { sc: 420 - ranges.length * 22 - (s.length - q.length) * 0.2, ranges }
}
function segsOf(text, ranges) {
  if (!ranges || !ranges.length) return [{ t: text, hit: false }]
  const out = []; let p = 0
  for (const [a, b] of ranges) { if (a > p) out.push({ t: text.slice(p, a), hit: false }); out.push({ t: text.slice(a, b), hit: true }); p = b }
  if (p < text.length) out.push({ t: text.slice(p), hit: false })
  return out
}
function runSearch() {
  const q = searchQ.value.trim().toLowerCase(); sel.value = 0
  if (searchTimer) clearTimeout(searchTimer)
  if (!q) { results.value = []; return }
  // instant in-page fuzzy feedback (unchanged)
  const out = []
  for (const n of raw.nodes) { const m = fuzzy(q, n.label); if (m) out.push({ kind: 't', id: n.id, label: n.label, sub: `${n.kind} · ${(+n.heat).toFixed(1)}`, glyph: MARK[n.kind] || '·', color: KIND[n.kind] || '#f2a93e', segs: segsOf(n.label, m.ranges), sc: m.sc + heatFrac(n.heat) * 80 }) }
  for (const k of raw.kerns) { if (!k.named || k.id === '__all__') continue; const m = fuzzy(q, k.label); if (m) out.push({ kind: 'g', id: k.id, label: k.label, sub: `group · ${k.count}`, glyph: '▣', color: '#f2a93e', segs: segsOf(k.label, m.ranges), sc: m.sc + 30 }) }
  for (const l of raw.links) { if (!l.text) continue; const m = fuzzy(q, l.text); if (m) out.push({ kind: 'r', id: l.target, label: l.text, sub: l.kind, glyph: rk(l.kind).g, color: rk(l.kind).c, segs: segsOf(l.text, m.ranges), sc: m.sc - 40 }) }
  out.sort((a, b) => b.sc - a.sc)
  results.value = out.slice(0, 8)
  // semantic re-rank after a 250ms pause
  searchTimer = setTimeout(() => semanticSearch(searchQ.value.trim()), 250)
}

async function semanticSearch(q) {
  if (!q) return
  const seq = ++searchSeq
  try {
    const res = await fetch('/search?q=' + encodeURIComponent(q) + '&k=10')
    if (!res.ok) return                      // 503 (embed down) → keep fuzzy list
    const data = await res.json()
    if (seq !== searchSeq) return            // a newer query started — drop this
    if (searchQ.value.trim() !== q) return   // input changed under us
    results.value = (data.results || []).map(h => {
      const isReason = h.heat === undefined  // reason hits carry no heat
      const kindGlyph = isReason ? rk(h.kind).g : (MARK[h.kind] || '·')
      const kindColor = isReason ? rk(h.kind).c : (KIND[h.kind] || '#f2a93e')
      return {
        kind: isReason ? 'r' : 't',
        id: h.id,
        label: h.label,
        sub: `${h.kind} · ${(+h.score).toFixed(2)}`,
        glyph: kindGlyph,
        color: kindColor,
        segs: segsOf(h.label, []),
      }
    })
  } catch (_) { /* network error → keep fuzzy results */ }
}
function clearSearch() { searchQ.value = ''; results.value = []; sel.value = 0 }
function pick(res) {
  if (!res) return
  if (res.kind === 'g') gotoKern(res.id)
  else setAnchor(res.id, true, true)
  clearSearch(); searchEl.value?.blur()
}

// ---- load --------------------------------------------------------------------
async function load() {
  try {
    raw = await (await fetch('/graph')).json()
    kernsById = {}; entsByKern = new Map(); nodeById = {}; adj = new Map()
    for (const k of raw.kerns) kernsById[k.id] = k
    for (const e of raw.nodes) { nodeById[e.id] = e; if (!entsByKern.has(e.kern)) entsByKern.set(e.kern, []); entsByKern.get(e.kern).push(e) }
    const push = (a, b, kind, dir, text, score) => { if (!adj.has(a)) adj.set(a, []); adj.get(a).push({ id: b, kind, dir, text, score }) }
    for (const l of raw.links) { push(l.source, l.target, l.kind, 'out', l.text, l.score); push(l.target, l.source, l.kind, 'in', l.text, l.score) }

    nThoughts.value = raw.nodes.length
    nGroups.value = raw.kerns.filter(k => k.id !== '__all__').length
    nDaemons.value = raw.daemons || 1

    const topo = raw.nodes.length + ':' + raw.kerns.length
    if (topo !== lastTopo) {
      lastTopo = topo; treeData = buildTree()
      meanHeat = {}; const reg = x => { if (x.type === 'kern') { meanHeat[x.id] = meanHeatOf(x); for (const c of x.children || []) reg(c) } }; reg(treeData)
      const ids = sphereStack.map(n => n.id); sphereStack = [treeData]
      for (let i = 1; i < ids.length; i++) { const n = findById(treeData, ids[i]); if (n && n.type === 'kern') sphereStack.push(n); else break }
      relayoutSphere()
    }
    if (anchorId && nodeById[anchorId]) { anchor.value = nodeById[anchorId]; relayoutReason() }
    else if (!anchorId) { const hot = raw.nodes.slice().sort((a, b) => (b.heat || 0) - (a.heat || 0))[0]; if (hot) setAnchor(hot.id, false, true) }
    if (searchQ.value) runSearch()
    err.value = ''
    fresh.value = true; if (freshT) clearTimeout(freshT); freshT = setTimeout(() => (fresh.value = false), 1100)
  } catch (e) { err.value = String(e) }
}

// ---- keyboard: type anywhere to find, digits to walk -------------------------
function onKey(ev) {
  const inInput = document.activeElement === searchEl.value
  if (ev.key === 'Escape') { if (results.value.length || searchQ.value) { clearSearch(); searchEl.value?.blur() } else sphereOut(); return }
  if (results.value.length) {
    if (ev.key === 'ArrowDown') { ev.preventDefault(); sel.value = (sel.value + 1) % results.value.length; return }
    if (ev.key === 'ArrowUp') { ev.preventDefault(); sel.value = (sel.value - 1 + results.value.length) % results.value.length; return }
    if (ev.key === 'Enter') { ev.preventDefault(); pick(results.value[sel.value]); return }
  }
  if (inInput) return
  if (ev.key === '/') { ev.preventDefault(); searchEl.value?.focus(); return }
  if (/^[1-4]$/.test(ev.key)) { const t = reasonSlots.value[+ev.key - 1]; if (t) setAnchor(t.ref.eid); return }
  if (ev.key.length === 1 && !ev.ctrlKey && !ev.metaKey && !ev.altKey) { ev.preventDefault(); searchEl.value?.focus(); searchQ.value += ev.key; runSearch() }
}

onMounted(() => { window.addEventListener('keydown', onKey); load(); timer = setInterval(load, 5000) })
onBeforeUnmount(() => { if (timer) clearInterval(timer); if (freshT) clearTimeout(freshT); if (searchTimer) clearTimeout(searchTimer); window.removeEventListener('keydown', onKey) })
</script>

<template>
  <div class="grain"></div>

  <div class="app">
    <!-- rail: identity + live pulse + heat legend -->
    <header class="rail">
      <div class="brand"><b>kern</b><span class="sub">living memory</span></div>
      <div class="rstats">
        <span class="dot" :class="{ on: fresh }"></span>
        <span>{{ nThoughts }} thoughts</span><i>·</i><span>{{ nGroups }} groups</span>
        <template v-if="nDaemons > 1"><i>·</i><span class="multi">{{ nDaemons }} daemons</span></template>
        <span v-if="err" class="err">— {{ err }}</span>
      </div>
      <div class="legend"><span>cold</span><div class="legbar"></div><span>hot</span></div>
    </header>

    <!-- hero: the anchor — one thought, calmly present -->
    <section class="hero" :class="{ empty: !anchor }" :key="anchorId">
      <template v-if="anchor">
        <div class="hglyph" :style="{ color: KIND[anchor.kind] || '#98c379' }">{{ MARK[anchor.kind] || '·' }}</div>
        <div class="hbody">
          <div class="hlabel">{{ anchor.label }}</div>
          <div class="hmeta">
            <span>{{ anchor.kind }}</span><i>·</i>
            <a class="hgroup" @click="gotoAnchorGroup">{{ sphereName(anchor.kern) }} ↗</a><i>·</i>
            <span>heat {{ (+anchor.heat).toFixed(1) }}</span><i>·</i>
            <span>{{ Math.round((anchor.conf || 0) * 100) }}% confident</span><i>·</i>
            <span>{{ reasonCount }} {{ reasonCount === 1 ? 'reason' : 'reasons' }}</span>
          </div>
        </div>
        <a class="hback" v-if="anchorHist.length" @click="anchorBack">↩</a>
      </template>
      <div v-else class="hprompt">Start typing to find any thought.</div>
    </section>

    <!-- two calm views -->
    <div class="stage">
      <section class="panel">
        <header class="phead">
          <span class="ptitle">structure</span>
          <span class="crumbs">
            <template v-for="(c, i) in sphereCrumbs" :key="c.id">
              <a @click="goCrumb(c.id)" :class="{ here: i === sphereCrumbs.length - 1 }">{{ c.label }}</a>
              <span v-if="i < sphereCrumbs.length - 1" class="sep">›</span>
            </template>
          </span>
          <span class="count">{{ Math.min(4, sphereTotal) }} / {{ sphereTotal }}</span>
          <a class="back" v-if="sphereStack.length > 1" @click="sphereOut">↑</a>
        </header>
        <div class="bwrap" @wheel.prevent="wheel('sphere', $event)">
          <div class="bento" :key="sphereKey">
            <div v-for="s in sphereSlots" :key="s.ref.id" class="tile" :class="[s.cls, s.ref.type, { anchored: s.ref.eid && s.ref.eid === anchorId }]"
              :style="{ background: fillOf(s.ref), color: textColor(fillOf(s.ref)), '--glow': fillOf(s.ref) }"
              @click="sphereClick(s.ref)" @mouseenter="detail = s.ref.label" @mouseleave="detail = ''">
              <span class="tmark">{{ s.ref.type === 'kern' ? '▣' : (MARK[s.ref.kind] || '·') }}</span>
              <span v-if="s.cls === 's4' && sphereTotal > 4" class="more">+{{ sphereTotal - 4 }}</span>
              <div class="tname">{{ s.ref.label }}</div>
              <div class="tmeta">{{ tileMeta(s.ref) }}</div>
            </div>
            <div v-if="!sphereSlots.length" class="empty">nothing here</div>
          </div>
        </div>
      </section>

      <section class="panel">
        <header class="phead">
          <span class="ptitle">reasons</span>
          <span class="atag" v-if="anchor">why this matters</span>
          <span class="count" v-if="anchor">{{ Math.min(4, reasonTotal) }} / {{ reasonTotal }}</span>
          <a class="back" v-if="anchorHist.length" @click="anchorBack">↩</a>
        </header>
        <div class="bwrap" @wheel.prevent="wheel('reason', $event)">
          <div class="bento" :key="reasonKey">
            <div v-for="s in reasonSlots" :key="s.ref.id" class="tile entity" :class="[s.cls, { anchored: s.ref.eid === anchorId }]"
              :style="{ background: fillOf(s.ref), color: textColor(fillOf(s.ref)), '--glow': fillOf(s.ref) }"
              @click="setAnchor(s.ref.eid)" @mouseenter="detail = s.ref.label" @mouseleave="detail = ''">
              <span class="tkey">{{ s.n }}</span>
              <span v-if="s.cls === 's4' && reasonTotal > 4" class="more">+{{ reasonTotal - 4 }}</span>
              <div class="tedge" :style="{ color: rk(s.ref.edge.kind).c }">{{ rk(s.ref.edge.kind).g }} {{ s.ref.edge.kind }} {{ s.ref.edge.dir === 'out' ? '→' : '←' }}</div>
              <div class="tname">{{ s.ref.label }}</div>
              <div class="tmeta">heat {{ (+s.ref.heat).toFixed(1) }} · {{ sphereName(s.ref.kern) }}</div>
            </div>
            <div v-if="anchor && !reasonSlots.length" class="empty">no reason edges yet</div>
            <div v-if="!anchor" class="empty">pick a thought to see why it matters</div>
          </div>
        </div>
      </section>
    </div>

    <!-- finder -->
    <div class="omni">
      <div v-if="results.length" class="results">
        <div v-for="(r, i) in results" :key="r.kind + r.id + i" class="rsl" :class="{ sel: i === sel }" @click="pick(r)" @mouseenter="sel = i">
          <span class="rslk" :style="{ color: r.color }">{{ r.glyph }}</span>
          <span class="rslt"><template v-for="(g, gi) in r.segs" :key="gi"><b v-if="g.hit">{{ g.t }}</b><template v-else>{{ g.t }}</template></template></span>
          <span class="rsls">{{ r.sub }}</span>
        </div>
      </div>
      <div class="ombar" :class="{ active: omActive }">
        <span class="omk">⌕</span>
        <input ref="searchEl" v-model="searchQ" @input="runSearch" @focus="omActive = true" @blur="omActive = false"
          placeholder="type to find any thought…" />
        <span class="omhint" v-if="detail">{{ detail }}</span>
        <span class="omkeys" v-else><kbd>↑↓</kbd> select <kbd>↵</kbd> go <kbd>1–4</kbd> walk <kbd>/</kbd> focus</span>
      </div>
    </div>
  </div>
</template>

<style>
:root {
  --ink: #f4f1ea; --muted: #8b8678; --line: rgba(244,241,234,0.10); --panel: rgba(244,241,234,0.018);
  --accent: #f2a93e;
  --display: 'Bricolage Grotesque', system-ui, sans-serif;
  --body: 'Hanken Grotesk', system-ui, sans-serif;
  --mono: 'IBM Plex Mono', ui-monospace, monospace;
}
* { box-sizing: border-box; }
html, body, #app { height: 100%; margin: 0; }

.app { height: 100%; display: flex; flex-direction: column; gap: 16px; padding: 18px 26px 16px;
  background: radial-gradient(120% 80% at 50% -8%, #16130f 0%, #0a0a0c 52%, #08080a 100%); color: var(--ink); }

/* ---- rail ---- */
.rail { display: flex; align-items: center; gap: 20px; flex: none; }
.brand { display: flex; align-items: baseline; gap: 9px; }
.brand b { font-family: var(--display); font-weight: 800; font-size: 22px; letter-spacing: -0.03em; }
.brand .sub { font-family: var(--mono); font-size: 10px; letter-spacing: .22em; text-transform: uppercase; color: var(--muted); }
.rstats { display: flex; align-items: center; gap: 8px; font-family: var(--mono); font-size: 11px; color: var(--muted); }
.rstats i { color: #3a3630; } .rstats .multi { color: var(--accent); } .rstats .err { color: #e8705e; }
.dot { width: 7px; height: 7px; border-radius: 50%; background: #2f2b24; transition: background .4s, box-shadow .4s; }
.dot.on { background: #98c379; box-shadow: 0 0 10px 1px #98c37988; }
.legend { margin-left: auto; display: flex; align-items: center; gap: 8px; font-family: var(--mono);
  font-size: 9px; letter-spacing: .14em; text-transform: uppercase; color: var(--muted); }
.legbar { width: 120px; height: 6px; border-radius: 4px; box-shadow: inset 0 0 0 1px var(--line);
  background: linear-gradient(90deg, #2a1809, #7c3c17, #cf6f25, #f2a93e, #ffe2a6); }

/* ---- hero ---- */
.hero { flex: none; display: flex; align-items: center; gap: 22px; padding: 22px 26px; min-height: 104px;
  border-radius: 18px; position: relative; overflow: hidden;
  background: linear-gradient(100deg, rgba(242,169,62,0.05), rgba(244,241,234,0.015) 60%);
  box-shadow: inset 0 0 0 1px rgba(242,169,62,0.14), 0 18px 50px -36px #000;
  animation: heroIn .42s cubic-bezier(.2,.8,.2,1); }
@keyframes heroIn { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: none; } }
.hero.empty { box-shadow: inset 0 0 0 1px var(--line); background: var(--panel); }
.hglyph { font-size: 34px; line-height: 1; flex: none; opacity: .85; }
.hbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 9px; }
.hlabel { font-family: var(--display); font-weight: 800; font-size: clamp(21px, 2.6vw, 32px); line-height: 1.08;
  letter-spacing: -0.022em; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.hmeta { display: flex; align-items: center; gap: 9px; flex-wrap: wrap; font-family: var(--mono); font-size: 11px;
  letter-spacing: .03em; color: var(--muted); }
.hmeta i { color: #3a3630; }
.hgroup { color: var(--accent); cursor: pointer; opacity: .9; } .hgroup:hover { opacity: 1; text-decoration: underline; }
.hback { flex: none; align-self: flex-start; font-size: 16px; color: var(--muted); cursor: pointer; padding: 2px 6px; border-radius: 8px; }
.hback:hover { color: var(--accent); background: rgba(242,169,62,0.1); }
.hprompt { font-family: var(--body); font-size: 17px; color: var(--muted); }

/* ---- stage / panels ---- */
.stage { flex: 1; min-height: 0; display: flex; gap: 20px; }
.panel { flex: 1; min-width: 0; display: flex; flex-direction: column; position: relative;
  border-radius: 18px; background: var(--panel); box-shadow: inset 0 0 0 1px rgba(244,241,234,0.055); overflow: hidden; }
.panel::before { content: ''; position: absolute; inset: 0; border-radius: 18px; pointer-events: none; z-index: 3;
  background: linear-gradient(180deg, rgba(244,241,234,0.045), transparent 13%); }
.phead { font-family: var(--mono); font-size: 11px; color: var(--ink); padding: 14px 17px 12px;
  border-bottom: 1px solid var(--line); display: flex; align-items: baseline; gap: 11px; }
.ptitle { letter-spacing: .2em; text-transform: uppercase; flex: none; }
.crumbs { display: flex; gap: 5px; align-items: baseline; overflow: hidden; }
.crumbs a { color: var(--muted); cursor: pointer; font-size: 10px; white-space: nowrap; max-width: 140px; overflow: hidden; text-overflow: ellipsis; }
.crumbs a:hover { color: var(--ink); } .crumbs a.here { color: var(--accent); } .crumbs .sep { color: #3a3630; }
.atag { color: #5d584e; font-size: 10px; letter-spacing: .04em; }
.count { margin-left: auto; flex: none; font-size: 10px; color: #5d584e; }
.back { flex: none; color: var(--muted); cursor: pointer; font-size: 13px; line-height: 1; }
.back:hover { color: var(--accent); }

.bwrap { flex: 1; overflow: hidden; }
.bento { height: 100%; display: grid; gap: 16px; padding: 18px;
  grid-template-columns: 1.45fr 1fr; grid-template-rows: repeat(3, 1fr); animation: bfade .3s ease; }
@keyframes bfade { from { opacity: 0; } to { opacity: 1; } }
@keyframes tile-in { from { opacity: 0; transform: translateY(14px) scale(.965); } to { opacity: 1; transform: none; } }
.s1 { grid-column: 1; grid-row: 1 / 4; }
.s2 { grid-column: 2; grid-row: 1; }
.s3 { grid-column: 2; grid-row: 2; }
.s4 { grid-column: 2; grid-row: 3; }

.tile { position: relative; isolation: isolate; border-radius: 16px; padding: 20px; overflow: hidden;
  display: flex; flex-direction: column; justify-content: flex-end; gap: 8px; cursor: pointer;
  box-shadow: inset 0 0 0 1px rgba(255,255,255,0.08), 0 12px 30px -20px rgba(0,0,0,0.8);
  transition: transform .16s cubic-bezier(.2,.7,.2,1), filter .16s, box-shadow .16s;
  animation: tile-in .5s cubic-bezier(.2,.8,.2,1) backwards; }
.tile.s1 { animation-delay: 0s; } .tile.s2 { animation-delay: .05s; }
.tile.s3 { animation-delay: .1s; } .tile.s4 { animation-delay: .15s; }
.tile::after { content: ''; position: absolute; inset: 0; z-index: 0; pointer-events: none; border-radius: inherit;
  background: radial-gradient(130% 90% at 82% -4%, rgba(255,255,255,0.15), transparent 50%); }
.tile > div, .tile > span { position: relative; z-index: 1; }
.tile:hover { transform: translateY(-3px); filter: brightness(1.06) saturate(1.03);
  box-shadow: inset 0 0 0 1px rgba(255,255,255,0.5), 0 22px 44px -20px rgba(0,0,0,0.85), 0 0 42px -12px var(--glow); }
.tile.anchored { box-shadow: inset 0 0 0 2.5px var(--accent), 0 0 36px -12px var(--accent), 0 22px 44px -20px rgba(0,0,0,0.85); }
.tmark { position: absolute; top: 15px; left: 18px; font-size: 13px; opacity: .55; }
.tkey { position: absolute; top: 12px; left: 14px; z-index: 2; font-family: var(--mono); font-size: 10px; font-weight: 600;
  width: 18px; height: 18px; display: grid; place-items: center; border-radius: 6px;
  background: rgba(0,0,0,0.3); box-shadow: inset 0 0 0 1px rgba(255,255,255,0.25); opacity: .8; }
.tedge { font-family: var(--mono); font-size: 10px; letter-spacing: .07em; text-transform: uppercase; color: inherit; opacity: .82; }
.s1 .tedge { font-size: 12px; }
.tname { font-family: var(--display); font-weight: 800; line-height: 1.08; letter-spacing: -0.02em; color: inherit;
  font-size: 16px; display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
.s1 .tname { font-size: 28px; -webkit-line-clamp: 7; }
.tmeta { font-family: var(--mono); font-size: 10px; letter-spacing: .08em; text-transform: uppercase; color: inherit; opacity: .64; }
.s1 .tmeta { font-size: 11px; }
.more { position: absolute; top: 12px; right: 14px; z-index: 2; font-family: var(--mono); font-size: 11px;
  background: rgba(0,0,0,0.3); color: #fff; padding: 3px 8px; border-radius: 10px; }
.empty { grid-column: 1 / -1; grid-row: 1 / -1; display: flex; align-items: center; justify-content: center;
  color: var(--muted); font-family: var(--body); font-size: 14px; }

/* ---- finder ---- */
.omni { flex: none; position: relative; }
.results { position: absolute; left: 0; right: 0; bottom: calc(100% + 8px); background: #14120f; border-radius: 14px; overflow: hidden;
  box-shadow: inset 0 0 0 1px var(--line), 0 24px 60px -24px rgba(0,0,0,0.92); animation: bfade .14s ease; }
.rsl { display: flex; align-items: center; gap: 13px; padding: 12px 17px; cursor: pointer; border-left: 2px solid transparent; }
.rsl + .rsl { border-top: 1px solid var(--line); }
.rsl.sel { background: rgba(242,169,62,0.1); border-left-color: var(--accent); }
.rslk { flex: none; width: 16px; text-align: center; font-size: 13px; }
.rslt { flex: 1; color: var(--muted); font-family: var(--body); font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rsl.sel .rslt { color: var(--ink); }
.rslt b { color: var(--ink); font-weight: 700; }
.rsls { font-family: var(--mono); font-size: 10px; color: #5d584e; flex: none; text-transform: uppercase; letter-spacing: .06em; }
.ombar { display: flex; align-items: center; gap: 13px; background: #14120f; border-radius: 14px; padding: 0 18px;
  box-shadow: inset 0 0 0 1px var(--line), 0 18px 50px -24px rgba(0,0,0,0.9); transition: box-shadow .2s; }
.ombar.active { box-shadow: inset 0 0 0 1px rgba(242,169,62,0.5), 0 0 0 3px rgba(242,169,62,0.08), 0 18px 50px -24px rgba(0,0,0,0.9); }
.omk { color: var(--accent); font-size: 16px; opacity: .8; }
.ombar input { flex: 1; min-width: 0; background: none; border: 0; outline: none; color: var(--ink); font-family: var(--body); font-size: 15px; padding: 16px 0; }
.ombar input::placeholder { color: #4f4b43; }
.omhint { flex: none; max-width: 40%; color: var(--muted); font-family: var(--body); font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.omkeys { flex: none; display: flex; align-items: center; gap: 5px; color: #5d584e; font-family: var(--mono); font-size: 10px; letter-spacing: .04em; }
.omkeys kbd { font-family: var(--mono); font-size: 10px; color: var(--muted); background: rgba(244,241,234,0.06);
  border-radius: 5px; padding: 2px 5px; box-shadow: inset 0 0 0 1px var(--line); margin-left: 6px; }
.omkeys kbd:first-child { margin-left: 0; }

/* ---- grain ---- */
.grain { position: fixed; inset: 0; z-index: 60; pointer-events: none; opacity: .05; mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='160' height='160'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E"); }
</style>
