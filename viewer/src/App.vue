<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import * as d3 from 'd3'

const crumbs = ref([])
const stats = ref('loading…')
const err = ref('')
const detail = ref('')
const mode = ref('cube')      // 'cube' | 'list'
const tiles = ref([])
const listItems = ref([])
const side = ref(480)
const stageEl = ref(null)

let timer = null
let raw = { nodes: [], links: [], kerns: [] }
let kernsById = {}, entsByKern = new Map(), meanHeat = {}
let treeData = null
let stack = []
let lastTopo = ''

const KIND = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }
const MARK = { Fact: '◆', Document: '■', Question: '▲', Claim: '●' }

function rootId() { const r = raw.kerns.find(k => !k.parent) || raw.kerns[0]; return r ? r.id : null }
function buildTree() {
  const make = (kid, seen) => {
    const k = kernsById[kid]; if (!k || seen.has(kid)) return null
    seen.add(kid)
    const node = { id: kid, label: k.named ? k.label : '(unnamed)', type: 'kern', children: [] }
    for (const c of k.children || []) { const cn = make(c, seen); if (cn) node.children.push(cn) }
    for (const e of entsByKern.get(kid) || [])
      node.children.push({ id: e.id, label: e.label, type: 'entity', kind: e.kind, heat: e.heat, conf: e.conf })
    return node
  }
  return make(rootId(), new Set()) || { id: 'root', label: 'root', type: 'kern', children: [] }
}
function d3Count(d) { if (d.type === 'entity') return 1; let n = 0; const w = x => { if (x.type === 'entity') n++; else for (const c of x.children || []) w(c) }; w(d); return n || 1 }
function subSpheres(d) { return (d.children || []).filter(c => c.type === 'kern').length }
function meanHeatOf(node) { let s = 0, n = 0; const w = x => { if (x.type === 'entity') { s += +x.heat || 0; n++ } else for (const c of x.children || []) w(c) }; w(node); return n ? s / n : 0 }
function findPath(node, id, acc = []) { acc.push(node); if (node.id === id) return acc.slice(); for (const c of node.children || []) { const r = findPath(c, id, acc); if (r) return r } acc.pop(); return null }
function findById(n, id) { if (n.id === id) return n; for (const c of n.children || []) { const r = findById(c, id); if (r) return r } return null }

const heatMax = () => Math.max(0.5, d3.max(raw.nodes, n => +n.heat || 0) || 1)
function ramp(h) { return d3.interpolateInferno(0.22 + 0.7 * Math.sqrt(Math.min(1, (h || 0) / heatMax()))) }
function fill(ref) { return ramp(ref.type === 'entity' ? ref.heat : meanHeat[ref.id]) }
function meta(ref) { return ref.type === 'kern' ? `${d3Count(ref)} thoughts${subSpheres(ref) ? ` · ${subSpheres(ref)} spheres` : ''}` : ref.kind }
function info(ref) { return ref.type === 'entity' ? `${ref.kind} · heat ${(+ref.heat).toFixed(2)} · conf ${(+ref.conf).toFixed(2)} — ${ref.label}` : `${ref.label} · ${d3Count(ref)} thoughts — click to enter` }

function relayout() {
  const cur = stack[stack.length - 1]
  crumbs.value = stack.map((n, i) => ({ id: n.id, label: i === 0 ? 'root' : n.label }))
  const kids = cur.children || []
  const kernKids = kids.filter(c => c.type === 'kern')

  // Last level (no sub-spheres) → a scrollable list of thoughts, not a blob.
  if (kernKids.length === 0) {
    mode.value = 'list'
    listItems.value = kids.filter(c => c.type === 'entity').sort((a, b) => (b.heat || 0) - (a.heat || 0))
    stats.value = `${raw.nodes.length} thoughts · ${raw.kerns.length} spheres · here: ${listItems.value.length}`
    return
  }

  // Otherwise → squarified cube of sub-topics (+ any loose thoughts), centered.
  mode.value = 'cube'
  const s = Math.max(160, Math.min(stageEl.value.clientWidth, stageEl.value.clientHeight) - 24)
  side.value = s
  const data = kids.map(c => ({ ref: c, value: d3Count(c) }))
  const r = d3.hierarchy({ children: data }).sum(d => d.value || 0).sort((a, b) => (b.value || 0) - (a.value || 0))
  d3.treemap().tile(d3.treemapSquarify.ratio(1)).size([s, s]).round(true).paddingInner(6)(r)
  tiles.value = (r.children || []).map(n => ({ ref: n.data.ref, x: n.x0, y: n.y0, w: n.x1 - n.x0, h: n.y1 - n.y0, n: n.value }))
  stats.value = `${raw.nodes.length} thoughts · ${raw.kerns.length} spheres · here: ${r.value}`
}

function enter(ref) { if (ref.type !== 'kern') return; const p = findPath(treeData, ref.id); if (p) { stack = p; relayout() } }
function out() { if (stack.length > 1) { stack.pop(); relayout() } }
function goTo(id) { const i = stack.findIndex(n => n.id === id); if (i >= 0) { stack.length = i + 1; relayout() } }
function onKey(ev) { if (ev.key === 'Escape') { ev.preventDefault(); out() } }

async function load() {
  try {
    raw = await (await fetch('/graph')).json()
    kernsById = {}; entsByKern = new Map()
    for (const k of raw.kerns) kernsById[k.id] = k
    for (const e of raw.nodes) { if (!entsByKern.has(e.kern)) entsByKern.set(e.kern, []); entsByKern.get(e.kern).push(e) }
    const topo = raw.nodes.length + ':' + raw.kerns.length
    if (topo !== lastTopo) {
      lastTopo = topo; treeData = buildTree()
      meanHeat = {}; const reg = x => { if (x.type === 'kern') { meanHeat[x.id] = meanHeatOf(x); for (const c of x.children || []) reg(c) } }; reg(treeData)
      const ids = stack.map(n => n.id); stack = [treeData]
      for (let i = 1; i < ids.length; i++) { const n = findById(treeData, ids[i]); if (n) stack.push(n); else break }
      relayout()
    }
    err.value = ''
  } catch (e) { err.value = String(e) }
}

onMounted(() => {
  window.addEventListener('keydown', onKey)
  window.addEventListener('resize', () => relayout())
  load(); timer = setInterval(load, 5000)
})
onBeforeUnmount(() => { if (timer) clearInterval(timer); window.removeEventListener('keydown', onKey) })
</script>

<template>
  <div class="hud">
    <b>kern</b>
    <span class="crumbs">
      <template v-for="(c, i) in crumbs" :key="c.id">
        <a @click="goTo(c.id)" :class="{ here: i === crumbs.length - 1 }">{{ c.label }}</a>
        <span v-if="i < crumbs.length - 1" class="sep"> › </span>
      </template>
    </span>
    <span class="stat">· {{ stats }}</span>
    <span v-if="err" class="err"> — {{ err }}</span>
  </div>

  <div ref="stageEl" class="stage">
    <div v-if="mode === 'cube'" class="cube" :style="{ width: side + 'px', height: side + 'px' }">
      <div v-for="t in tiles" :key="t.ref.id" class="tile" :class="t.ref.type"
        :style="{ left: t.x + 'px', top: t.y + 'px', width: t.w + 'px', height: t.h + 'px', background: fill(t.ref) }"
        @click="enter(t.ref)" @mouseenter="detail = info(t.ref)" @mouseleave="detail = ''">
        <div class="tname">{{ t.ref.label }}</div>
        <div class="tmeta">{{ meta(t.ref) }}</div>
      </div>
    </div>

    <div v-else class="list">
      <div class="lhead">{{ listItems.length }} thoughts in this sphere</div>
      <div v-for="e in listItems" :key="e.id" class="row"
        @mouseenter="detail = info(e)" @mouseleave="detail = ''">
        <span class="rk" :style="{ color: KIND[e.kind] || '#98c379' }">{{ MARK[e.kind] || '·' }}</span>
        <span class="rt">{{ e.label }}</span>
        <span class="rbar"><i :style="{ width: Math.min(100, (e.heat / heatMax()) * 100) + '%', background: fill(e) }"></i></span>
      </div>
    </div>
  </div>

  <div class="path">{{ detail || 'click a cube to enter · Esc to go back' }}</div>
</template>

<style>
* { box-sizing: border-box; }
html, body, #app { height: 100%; margin: 0; }
.stage { position: fixed; inset: 0; background: #07090d; display: flex; align-items: center; justify-content: center; }

.cube { position: relative; }
.tile { position: absolute; border-radius: 8px; overflow: hidden; padding: 8px;
  display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center;
  border: 1px solid rgba(255,255,255,0.08); cursor: pointer; transition: filter .1s, transform .1s; }
.tile.kern:hover { filter: brightness(1.18); transform: scale(1.01); border-color: #fff; z-index: 2; }
.tile.entity { cursor: default; }
.tname { font: 600 13px/1.25 system-ui, sans-serif; color: #fff;
  text-shadow: 0 1px 3px rgba(0,0,0,.8); display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
.tmeta { margin-top: 5px; font: 11px system-ui, sans-serif; color: rgba(255,255,255,.8); text-shadow: 0 1px 2px rgba(0,0,0,.8); }

.list { width: min(760px, 94vw); height: calc(100vh - 110px); overflow-y: auto; padding: 6px 4px 30px; }
.lhead { color: #6b7682; font: 12px system-ui, sans-serif; padding: 4px 8px 10px; }
.row { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 7px;
  border: 1px solid #161c24; margin-bottom: 6px; background: #0c1014; }
.row:hover { background: #11161c; border-color: #243040; }
.rk { font-size: 13px; flex: none; width: 14px; text-align: center; }
.rt { flex: 1; color: #d6dde4; font: 13px/1.4 system-ui, sans-serif; }
.rbar { flex: none; width: 70px; height: 6px; background: #161c24; border-radius: 3px; overflow: hidden; }
.rbar i { display: block; height: 100%; border-radius: 3px; }

.hud { position: fixed; top: 12px; left: 14px; z-index: 10; background: #11151ad9; color: #cdd3da;
  padding: 8px 13px; border-radius: 9px; border: 1px solid #1d2530; backdrop-filter: blur(6px);
  font: 13px system-ui, sans-serif; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; max-width: 92vw; }
.hud b { color: #7fd1ae; letter-spacing: .4px; }
.crumbs a { color: #9ec1e0; cursor: pointer; }
.crumbs a.here { color: #f0c987; font-weight: 600; }
.crumbs .sep { color: #46505c; }
.stat { color: #8a96a2; }
.err { color: #e06c75; }
.path { position: fixed; bottom: 12px; left: 14px; right: 14px; z-index: 10; color: #cfd6de;
  font: 13px system-ui, sans-serif; background: #11151ad9; backdrop-filter: blur(6px);
  padding: 7px 13px; border-radius: 9px; border: 1px solid #1d2530;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
