<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import * as d3 from 'd3'

// Two panels, each a fixed 5-slot bento ranked by relevance (heat). Always 5
// readable things, never a fractal of tiny cells. Click / zoom a slot re-scopes:
// its top-5 children animate in. Left = sphere tree, right = reason ego-graph.
// Shared anchor links them. Bottom omnibar = search-to-anchor.

const stats = ref('loading…')
const err = ref('')
const detail = ref('')
const anchor = ref(null)
const searchQ = ref('')
const results = ref([])
const searchEl = ref(null)

const sphereSlots = ref([]), sphereExtra = ref(0), sphereCrumbs = ref([]), sphereKey = ref('')
const reasonSlots = ref([]), reasonExtra = ref(0), reasonKey = ref('')

let timer = null
let raw = { nodes: [], links: [], kerns: [] }
let kernsById = {}, entsByKern = new Map(), meanHeat = {}
let nodeById = {}, adj = new Map()
let treeData = null
let sphereStack = [], anchorHist = []
let lastTopo = ''
let anchorId = ''
let lastWheel = 0

const SLOTS = ['s1', 's2', 's3', 's4', 's5']
const KIND = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }
const MARK = { Fact: '◆', Document: '■', Question: '▲', Claim: '●' }
const RKIND = {
  Supersedes: { g: '↟', c: '#e06c75' }, Ratification: { g: '✓', c: '#98c379' },
  Question: { g: '?', c: '#c678dd' }, Similarity: { g: '≈', c: '#61afef' },
  Provenance: { g: '⌖', c: '#e5c07b' }, Rephrase: { g: '↺', c: '#56b6c2' },
  Spawn: { g: '✶', c: '#9aa0aa' },
}
function rk(k) { return RKIND[k] || { g: '·', c: '#9aa0aa' } }

// ---- tree + maps -------------------------------------------------------------
function rootId() { const r = raw.kerns.find(k => !k.parent) || raw.kerns[0]; return r ? r.id : null }
function buildTree() {
  const make = (kid, seen) => {
    const k = kernsById[kid]; if (!k || seen.has(kid)) return null
    seen.add(kid)
    const node = { id: kid, label: k.named ? k.label : '(unnamed)', type: 'kern', eid: null, children: [] }
    for (const c of k.children || []) { const cn = make(c, seen); if (cn) node.children.push(cn) }
    for (const e of entsByKern.get(kid) || [])
      node.children.push({ id: e.id, eid: e.id, label: e.label, type: 'entity', kind: e.kind, heat: e.heat, conf: e.conf, kern: kid })
    return node
  }
  return make(rootId(), new Set()) || { id: 'root', label: 'root', type: 'kern', eid: null, children: [] }
}
function d3Count(d) { if (d.type === 'entity') return 1; let n = 0; const w = x => { if (x.type === 'entity') n++; else for (const c of x.children || []) w(c) }; w(d); return n || 1 }
function meanHeatOf(node) { let s = 0, n = 0; const w = x => { if (x.type === 'entity') { s += +x.heat || 0; n++ } else for (const c of x.children || []) w(c) }; w(node); return n ? s / n : 0 }
function findPath(node, id, acc = []) { acc.push(node); if (node.id === id) return acc.slice(); for (const c of node.children || []) { const r = findPath(c, id, acc); if (r) return r } acc.pop(); return null }
function findById(n, id) { if (n.id === id) return n; for (const c of n.children || []) { const r = findById(c, id); if (r) return r } return null }
function sphereName(kid) { const k = kernsById[kid]; return k ? (k.named ? k.label : '(unnamed)') : '?' }
function relevance(n) { return n.type === 'entity' ? (+n.heat || 0) : (meanHeat[n.id] || 0) }

// ---- color -------------------------------------------------------------------
const heatMax = () => Math.max(0.5, d3.max(raw.nodes, n => +n.heat || 0) || 1)
const WARM = d3.interpolateRgbBasis(['#2a1809', '#7c3c17', '#cf6f25', '#f2a93e', '#ffe2a6'])
function ramp(h) { return WARM(0.12 + 0.85 * Math.sqrt(Math.min(1, (h || 0) / heatMax()))) }
function fillOf(n) { return ramp(n.type === 'entity' ? n.heat : (meanHeat[n.id] || 0)) }
function textColor(bg) { const c = d3.color(bg); if (!c) return '#fff'; return (0.299 * c.r + 0.587 * c.g + 0.114 * c.b) / 255 > 0.62 ? '#1c1206' : '#fdfaf3' }
function meta(n) { return n.type === 'kern' ? `${d3Count(n)} thoughts${(n.children || []).filter(c => c.type === 'kern').length ? ` · ${(n.children).filter(c => c.type === 'kern').length} groups` : ''}` : `${n.kind} · heat ${(+n.heat).toFixed(2)}` }

// ---- left panel: sphere bento ------------------------------------------------
function relayoutSphere() {
  const cur = sphereStack[sphereStack.length - 1]
  sphereKey.value = cur.id
  sphereCrumbs.value = sphereStack.map((n, i) => ({ id: n.id, label: i === 0 ? 'root' : n.label }))
  // a group shows its children; a thought shows its connected thoughts — so you
  // can keep clicking thought → thought and move through the graph.
  const kids = cur.type === 'entity' ? neighborsOf(cur.id) : (cur.children || [])
  const sorted = kids.slice().sort((a, b) => relevance(b) - relevance(a))
  sphereSlots.value = sorted.slice(0, 5).map((ref, i) => ({ ref, cls: SLOTS[i] }))
  sphereExtra.value = Math.max(0, sorted.length - 5)
  stats.value = `${raw.nodes.length} thoughts · ${raw.kerns.length} groups`
}
function sphereClick(ref) { sphereStack.push(ref); relayoutSphere() }
function sphereOut() { if (sphereStack.length > 1) { sphereStack.pop(); relayoutSphere() } }
function goCrumb(id) { const i = sphereStack.findIndex(n => n.id === id); if (i >= 0) { sphereStack.length = i + 1; relayoutSphere() } }

// ---- right panel: reason bento -----------------------------------------------
function neighborsOf(eid) {
  const seen = new Map()
  for (const e of (adj.get(eid) || [])) { const nb = nodeById[e.id]; if (!nb) continue; if (!seen.has(e.id)) seen.set(e.id, { ...nb, eid: nb.id, type: 'entity', edge: e }) }
  return [...seen.values()].sort((a, b) => (b.heat || 0) - (a.heat || 0))
}
function relayoutReason() {
  const a = anchor.value
  if (!a) { reasonSlots.value = []; reasonExtra.value = 0; reasonKey.value = ''; return }
  reasonKey.value = a.id
  const ns = neighborsOf(a.id)
  reasonSlots.value = ns.slice(0, 5).map((ref, i) => ({ ref, cls: SLOTS[i] }))
  reasonExtra.value = Math.max(0, ns.length - 5)
}
// the link between panels: focus a thought → reveal its sphere left, its reasons right.
// reasons panel is independent: anchoring re-roots it only, never the thoughts panel.
function setAnchor(eid, push = true) {
  const n = nodeById[eid]; if (!n) return
  if (push && anchorId && anchorId !== eid) anchorHist.push(anchorId)
  anchorId = eid; anchor.value = n
  relayoutReason()
}
function anchorBack() { const prev = anchorHist.pop(); if (prev && nodeById[prev]) setAnchor(prev, false) }

// ---- wheel = zoom in (descend top slot) / out (ascend) ----------------------
function wheel(panel, ev) {
  const now = performance.now(); if (now - lastWheel < 320) return; lastWheel = now
  if (panel === 'sphere') {
    if (ev.deltaY < 0) { const top = sphereSlots.value[0]; if (top) sphereClick(top.ref) }
    else sphereOut()
  } else {
    if (ev.deltaY < 0) { const top = reasonSlots.value[0]; if (top) setAnchor(top.ref.eid) }
    else anchorBack()
  }
}

// ---- search ------------------------------------------------------------------
function runSearch() {
  const q = searchQ.value.trim().toLowerCase()
  if (!q) { results.value = []; return }
  const t = raw.nodes.filter(n => n.label.toLowerCase().includes(q)).sort((a, b) => (b.heat || 0) - (a.heat || 0)).slice(0, 10)
    .map(n => ({ kind: 't', id: n.id, label: n.label, sub: `${n.kind} · ${(+n.heat).toFixed(2)}` }))
  const r = raw.links.filter(l => (l.text || '').toLowerCase().includes(q)).slice(0, 6)
    .map(l => ({ kind: 'r', id: l.target, label: l.text || '(reason)', sub: l.kind }))
  results.value = [...t, ...r]
}
function pick(res) { searchQ.value = ''; results.value = []; setAnchor(res.id) }

// ---- load --------------------------------------------------------------------
async function load() {
  try {
    raw = await (await fetch('/graph')).json()
    kernsById = {}; entsByKern = new Map(); nodeById = {}; adj = new Map()
    for (const k of raw.kerns) kernsById[k.id] = k
    for (const e of raw.nodes) { nodeById[e.id] = e; if (!entsByKern.has(e.kern)) entsByKern.set(e.kern, []); entsByKern.get(e.kern).push(e) }
    const push = (a, b, kind, dir, text, score) => { if (!adj.has(a)) adj.set(a, []); adj.get(a).push({ id: b, kind, dir, text, score }) }
    for (const l of raw.links) { push(l.source, l.target, l.kind, 'out', l.text, l.score); push(l.target, l.source, l.kind, 'in', l.text, l.score) }

    const topo = raw.nodes.length + ':' + raw.kerns.length
    if (topo !== lastTopo) {
      lastTopo = topo; treeData = buildTree()
      meanHeat = {}; const reg = x => { if (x.type === 'kern') { meanHeat[x.id] = meanHeatOf(x); for (const c of x.children || []) reg(c) } }; reg(treeData)
      const ids = sphereStack.map(n => n.id); sphereStack = [treeData]
      for (let i = 1; i < ids.length; i++) { const n = findById(treeData, ids[i]); if (n) sphereStack.push(n); else break }
      relayoutSphere()
    }
    if (anchorId && nodeById[anchorId]) { anchor.value = nodeById[anchorId]; relayoutReason() }
    else if (!anchorId) { const hot = raw.nodes.slice().sort((a, b) => (b.heat || 0) - (a.heat || 0))[0]; if (hot) setAnchor(hot.id, false) }
    err.value = ''
  } catch (e) { err.value = String(e) }
}

function onKey(ev) {
  if (ev.key === '/' && document.activeElement !== searchEl.value) { ev.preventDefault(); searchEl.value?.focus() }
  else if (ev.key === 'Escape') { if (results.value.length || searchQ.value) { searchQ.value = ''; results.value = []; searchEl.value?.blur() } else sphereOut() }
}

onMounted(() => {
  window.addEventListener('keydown', onKey)
  load(); timer = setInterval(load, 5000)
})
onBeforeUnmount(() => { if (timer) clearInterval(timer); window.removeEventListener('keydown', onKey) })
</script>

<template>
  <div class="grain"></div>
  <div class="hud">
    <b>kern</b><span class="stat">· {{ stats }}</span><span v-if="err" class="err"> — {{ err }}</span>
  </div>

  <div class="stage">
    <!-- RIGHT: thoughts bento (structural browse) -->
    <section class="panel" style="order:1">
      <header class="phead">
        thoughts
        <span class="crumbs">
          <template v-for="(c, i) in sphereCrumbs" :key="c.id">
            <a @click="goCrumb(c.id)" :class="{ here: i === sphereCrumbs.length - 1 }">{{ c.label }}</a>
            <span v-if="i < sphereCrumbs.length - 1" class="sep">›</span>
          </template>
        </span>
        <a class="back" v-if="sphereStack.length > 1" @click="sphereOut">↑ up</a>
      </header>
      <div class="bwrap" @wheel.prevent="wheel('sphere', $event)">
        <div class="bento" :key="sphereKey">
          <div v-for="s in sphereSlots" :key="s.ref.id" class="tile" :class="[s.cls, s.ref.type]"
            :style="{ background: fillOf(s.ref), color: textColor(fillOf(s.ref)), '--glow': fillOf(s.ref) }"
            @click="sphereClick(s.ref)" @mouseenter="detail = s.ref.label" @mouseleave="detail = ''">
            <span v-if="s.cls === 's5' && sphereExtra" class="more">+{{ sphereExtra }}</span>
            <div class="tname">{{ s.ref.label }}</div>
            <div class="tmeta">{{ meta(s.ref) }}</div>
          </div>
          <div v-if="!sphereSlots.length" class="empty">nothing here</div>
        </div>
      </div>
    </section>

    <!-- LEFT: reason bento (independent associative walk) -->
    <section class="panel" style="order:0">
      <header class="phead">
        reasons
        <span class="atag" v-if="anchor"><i :style="{ color: KIND[anchor.kind] || '#98c379' }">{{ MARK[anchor.kind] || '·' }}</i> {{ anchor.label }}</span>
        <a class="back" v-if="anchorHist.length" @click="anchorBack">↩ back</a>
      </header>
      <div class="bwrap" @wheel.prevent="wheel('reason', $event)">
        <div class="bento" :key="reasonKey">
          <div v-for="s in reasonSlots" :key="s.ref.id" class="tile entity" :class="[s.cls, { anchored: s.ref.eid === anchorId }]"
            :style="{ background: fillOf(s.ref), color: textColor(fillOf(s.ref)), '--glow': fillOf(s.ref) }"
            @click="setAnchor(s.ref.eid)" @mouseenter="detail = s.ref.label" @mouseleave="detail = ''">
            <span v-if="s.cls === 's5' && reasonExtra" class="more">+{{ reasonExtra }}</span>
            <div class="tedge" :style="{ color: rk(s.ref.edge.kind).c }">{{ rk(s.ref.edge.kind).g }} {{ s.ref.edge.kind }} {{ s.ref.edge.dir === 'out' ? '→' : '←' }}</div>
            <div class="tname">{{ s.ref.label }}</div>
            <div class="tmeta">heat {{ (+s.ref.heat).toFixed(2) }} · in {{ sphereName(s.ref.kern) }}</div>
          </div>
          <div v-if="anchor && !reasonSlots.length" class="empty">no reason edges from this thought</div>
          <div v-if="!anchor" class="empty">pick a thought to see its reasons</div>
        </div>
      </div>
    </section>
  </div>

  <div class="omni">
    <div v-if="results.length" class="results">
      <div v-for="r in results" :key="r.kind + r.id" class="rsl" @click="pick(r)">
        <span class="rslk" :class="r.kind">{{ r.kind === 't' ? '◆' : '≈' }}</span>
        <span class="rslt">{{ r.label }}</span><span class="rsls">{{ r.sub }}</span>
      </div>
    </div>
    <div class="ombar">
      <span class="omk">⌕</span>
      <input ref="searchEl" v-model="searchQ" @input="runSearch" placeholder="search thoughts + reasons to anchor…  ( / to focus · semantic soon )" />
      <span class="omhint">{{ detail }}</span>
    </div>
  </div>
</template>

<style>
:root {
  --ink: #f4f1ea; --muted: #8b8678; --line: rgba(244,241,234,0.10); --panel: rgba(244,241,234,0.018);
  --display: 'Bricolage Grotesque', system-ui, sans-serif;
  --body: 'Hanken Grotesk', system-ui, sans-serif;
  --mono: 'IBM Plex Mono', ui-monospace, monospace;
}
* { box-sizing: border-box; }
html, body, #app { height: 100%; margin: 0; }

.stage { position: fixed; inset: 60px 24px 76px; display: flex; gap: 18px;
  background: radial-gradient(120% 90% at 50% -10%, #16130f 0%, #0a0a0c 55%, #08080a 100%); }
.panel { flex: 1; min-width: 0; display: flex; flex-direction: column; position: relative;
  border-radius: 18px; background: var(--panel); box-shadow: inset 0 0 0 1px rgba(244,241,234,0.06); overflow: hidden; }
.panel::before { content: ''; position: absolute; inset: 0; border-radius: 18px; pointer-events: none; z-index: 2;
  background: linear-gradient(180deg, rgba(244,241,234,0.05), transparent 16%); }
.phead { font-family: var(--mono); font-size: 11px; letter-spacing: .16em; text-transform: uppercase;
  color: var(--ink); padding: 13px 16px 11px; border-bottom: 1px solid var(--line); display: flex; align-items: baseline; gap: 10px; }
.crumbs { display: flex; gap: 5px; align-items: baseline; overflow: hidden; }
.crumbs a { color: var(--muted); cursor: pointer; font-size: 10px; letter-spacing: .04em; white-space: nowrap;
  max-width: 120px; overflow: hidden; text-overflow: ellipsis; }
.crumbs a.here { color: #f2a93e; } .crumbs .sep { color: #3a3630; }
.atag { color: var(--muted); font-family: var(--body); font-size: 11px; letter-spacing: 0; text-transform: none;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 60%; }
.back { margin-left: auto; color: var(--muted); cursor: pointer; font-size: 10px; letter-spacing: .1em; flex: none; }
.back:hover { color: #f2a93e; }

.bwrap { flex: 1; overflow: hidden; }
.bento { height: 100%; display: grid; gap: 14px; padding: 16px;
  grid-template-columns: 1.5fr 1fr 1fr; grid-template-rows: 1fr 1fr 1fr 1fr;
  animation: bfade .3s ease; }
@keyframes bfade { from { opacity: 0; } to { opacity: 1; } }
@keyframes tile-in { from { opacity: 0; transform: translateY(14px) scale(.965); } to { opacity: 1; transform: none; } }
.s1 { grid-column: 1; grid-row: 1 / 5; }
.s2 { grid-column: 2 / 4; grid-row: 1 / 2; }
.s3 { grid-column: 2 / 4; grid-row: 2 / 3; }
.s4 { grid-column: 2 / 3; grid-row: 3 / 5; }
.s5 { grid-column: 3 / 4; grid-row: 3 / 5; }

.tile { position: relative; isolation: isolate; border-radius: 16px; padding: 18px; overflow: hidden;
  display: flex; flex-direction: column; justify-content: flex-end; gap: 8px; cursor: pointer;
  box-shadow: inset 0 0 0 1px rgba(255,255,255,0.08), 0 12px 30px -18px rgba(0,0,0,0.8);
  transition: transform .16s cubic-bezier(.2,.7,.2,1), filter .16s, box-shadow .16s;
  animation: tile-in .5s cubic-bezier(.2,.8,.2,1) backwards; }
.tile.s1 { animation-delay: 0s; } .tile.s2 { animation-delay: .05s; }
.tile.s3 { animation-delay: .1s; } .tile.s4 { animation-delay: .15s; } .tile.s5 { animation-delay: .2s; }
.tile::after { content: ''; position: absolute; inset: 0; z-index: 0; pointer-events: none; border-radius: inherit;
  background: radial-gradient(130% 90% at 82% -4%, rgba(255,255,255,0.16), transparent 48%); }
.tile > div { position: relative; z-index: 1; }
.tile:hover { transform: translateY(-3px); filter: brightness(1.07) saturate(1.04);
  box-shadow: inset 0 0 0 1px rgba(255,255,255,0.5), 0 22px 44px -18px rgba(0,0,0,0.85), 0 0 42px -10px var(--glow); }
.tile.anchored { box-shadow: inset 0 0 0 2.5px #f2a93e, 0 0 36px -10px #f2a93e, 0 22px 44px -18px rgba(0,0,0,0.85); }
.tname { font-family: var(--display); font-weight: 800; line-height: 1.07; letter-spacing: -0.02em; color: inherit;
  font-size: 16px; display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
.s1 .tname { font-size: 30px; -webkit-line-clamp: 8; }
.s2 .tname, .s3 .tname { font-size: 19px; -webkit-line-clamp: 3; }
.tmeta { font-family: var(--mono); font-size: 10px; letter-spacing: .12em; text-transform: uppercase; color: inherit; opacity: .68; }
.s1 .tmeta { font-size: 11px; }
.tedge { font-family: var(--mono); font-size: 10px; letter-spacing: .08em; text-transform: uppercase; color: inherit; opacity: .9; }
.s1 .tedge { font-size: 12px; }
.more { position: absolute; top: 12px; right: 14px; z-index: 2; font-family: var(--mono); font-size: 11px;
  background: rgba(0,0,0,0.32); color: #fff; padding: 3px 8px; border-radius: 10px; }
.empty { grid-column: 1 / -1; grid-row: 1 / -1; display: flex; align-items: center; justify-content: center;
  color: var(--muted); font-family: var(--body); font-size: 14px; }

.omni { position: fixed; left: 24px; right: 24px; bottom: 16px; z-index: 20; }
.results { background: #131210; border-radius: 12px; margin-bottom: 8px; overflow-y: auto; max-height: 44vh;
  box-shadow: inset 0 0 0 1px var(--line), 0 18px 50px -20px rgba(0,0,0,0.9); }
.rsl { display: flex; align-items: center; gap: 12px; padding: 11px 16px; cursor: pointer; border-bottom: 1px solid var(--line); }
.rsl:hover { background: rgba(244,241,234,0.05); }
.rslk { flex: none; width: 16px; text-align: center; } .rslk.t { color: #e5c07b; } .rslk.r { color: #61afef; }
.rslt { flex: 1; color: var(--ink); font-family: var(--body); font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rsls { font-family: var(--mono); font-size: 10px; color: var(--muted); flex: none; }
.ombar { display: flex; align-items: center; gap: 12px; background: #131210; border-radius: 12px; padding: 0 16px;
  box-shadow: inset 0 0 0 1px var(--line), 0 18px 50px -22px rgba(0,0,0,0.9); }
.omk { color: var(--muted); font-size: 15px; }
.ombar input { flex: 1; min-width: 0; background: none; border: 0; outline: none;
  color: var(--ink); font-family: var(--body); font-size: 14px; padding: 15px 0; }
.ombar input::placeholder { color: #4f4b43; }
.omhint { flex: none; max-width: 38%; color: var(--muted); font-family: var(--body); font-size: 13px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: right; }

.grain { position: fixed; inset: 0; z-index: 60; pointer-events: none; opacity: .05; mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='160' height='160'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E"); }

.hud { position: fixed; top: 20px; left: 28px; z-index: 10; color: var(--ink); font-family: var(--body); font-size: 13px;
  display: flex; gap: 10px; align-items: baseline; }
.hud b { font-family: var(--display); font-weight: 800; font-size: 17px; letter-spacing: -0.02em; }
.stat { font-family: var(--mono); font-size: 11px; letter-spacing: .06em; color: var(--muted); }
.err { color: #e8705e; }
</style>
