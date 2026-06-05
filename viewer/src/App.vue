<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import * as d3 from 'd3'

const crumbs = ref([])
const stats = ref('loading…')
const err = ref('')
const detail = ref('')
const svgEl = ref(null)

let timer = null
let raw = { nodes: [], links: [], kerns: [] }
let kernsById = {}, entsByKern = new Map()
let treeData = null
let stack = []            // focus path; last = current cube we're inside
let layoutRoot = null     // current d3 layout (for hit-testing)
let lastTopo = ''
let hoverId = null
let wheelLock = 0

const KIND = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }

function rootId() { const r = raw.kerns.find(k => !k.parent) || raw.kerns[0]; return r ? r.id : null }

function buildTree() {
  const make = (kid, seen) => {
    const k = kernsById[kid]; if (!k || seen.has(kid)) return null
    seen.add(kid)
    const node = { id: kid, label: k.named ? k.label : '(unnamed)', type: 'kern', children: [] }
    for (const c of k.children || []) { const cn = make(c, seen); if (cn) node.children.push(cn) }
    for (const e of entsByKern.get(kid) || [])
      node.children.push({ id: e.id, label: e.label, type: 'entity', kind: e.kind, heat: e.heat, conf: e.conf, value: 1 })
    return node
  }
  return make(rootId(), new Set()) || { id: 'root', label: 'root', type: 'kern', children: [] }
}
function findPath(node, id, acc = []) {
  acc.push(node); if (node.id === id) return acc.slice()
  for (const c of node.children || []) { const r = findPath(c, id, acc); if (r) return r }
  acc.pop(); return null
}

const sideOf = () => Math.max(120, Math.min(svgEl.value.clientWidth, svgEl.value.clientHeight) - 70)
let ox = 0, oy = 0

let svg, g
function layout() {
  const cur = stack[stack.length - 1]
  const side = sideOf()
  ox = (svgEl.value.clientWidth - side) / 2
  oy = (svgEl.value.clientHeight - side) / 2
  const r = d3.hierarchy(cur).sum(d => d.value || 0).sort((a, b) => (b.value || 0) - (a.value || 0))
  d3.treemap().tile(d3.treemapSquarify).size([side, side]).round(true)
    .paddingInner(3).paddingOuter(3).paddingTop(d => d.depth === 1 && d.data.type === 'kern' ? 16 : 0)(r)
  return r
}

function render() {
  crumbs.value = stack.map((n, i) => ({ id: n.id, label: i === 0 ? 'root' : n.label }))
  layoutRoot = layout()
  const r = layoutRoot
  g.attr('transform', `translate(${ox},${oy})`)
  g.selectAll('*').remove()

  const W = d => Math.max(0, d.x1 - d.x0), H = d => Math.max(0, d.y1 - d.y0)
  const cubes = (r.children || [])

  // inner preview cells (grandchildren) — the "smaller cubes" inside each cube.
  const inner = []
  for (const c of cubes) for (const gc of (c.children || [])) inner.push(gc)
  g.selectAll('rect.in').data(inner).join('rect').attr('class', 'in')
    .attr('x', d => d.x0).attr('y', d => d.y0).attr('width', W).attr('height', H)
    .attr('fill', d => d.data.type === 'entity' ? KIND[d.data.kind] || '#98c379' : '#3a4654')
    .attr('fill-opacity', d => d.data.type === 'entity' ? (0.3 + 0.6 * Math.min(1, (d.data.heat || 0) / 2)) : 0.5)
    .attr('rx', 1)

  // main topic cubes.
  const cube = g.selectAll('g.cube').data(cubes).join('g').attr('class', 'cube')
  cube.append('rect')
    .attr('x', d => d.x0).attr('y', d => d.y0).attr('width', W).attr('height', H)
    .attr('fill', d => d.data.type === 'entity' ? KIND[d.data.kind] || '#98c379' : 'none')
    .attr('fill-opacity', d => d.data.type === 'entity' ? (0.3 + 0.6 * Math.min(1, (d.data.heat || 0) / 2)) : 1)
    .attr('stroke', d => d.data.type === 'kern' ? '#7fd1ae' : '#06080b')
    .attr('stroke-opacity', d => d.data.type === 'kern' ? 0.55 : 1)
    .attr('stroke-width', d => d.data.id === hoverId ? 2 : 1)
    .attr('rx', 2)
  cube.filter(d => d.data.type === 'kern' && W(d) > 36)
    .append('text').attr('x', d => d.x0 + 5).attr('y', d => d.y0 + 11)
    .attr('fill', '#cfe9df').attr('font-size', '11px').attr('font-weight', 600)
    .text(d => clip(d.data.label + '  (' + d.value + ')', W(d)))

  stats.value = `${raw.nodes.length} thoughts · ${raw.kerns.length} spheres · here: ${r.value}`
}

function clip(s, px) { const n = Math.floor((px - 8) / 5.6); return n >= s.length ? s : (n > 1 ? s.slice(0, n - 1) + '…' : '') }

function hit(ev) {
  if (!layoutRoot) return null
  const [mx, my] = d3.pointer(ev, svgEl.value)
  const x = mx - ox, y = my - oy
  let leaf = null, cube = null
  for (const c of (layoutRoot.children || [])) {
    if (x >= c.x0 && x <= c.x1 && y >= c.y0 && y <= c.y1) {
      cube = c
      for (const gc of (c.children || [])) if (x >= gc.x0 && x <= gc.x1 && y >= gc.y0 && y <= gc.y1) leaf = gc
    }
  }
  return { cube, leaf }
}

function onMove(ev) {
  const h = hit(ev)
  const cube = h?.cube, leaf = h?.leaf
  const id = cube?.data.id || null
  if (id !== hoverId) {
    hoverId = id
    g.selectAll('g.cube rect').attr('stroke-width', d => d.data.id === hoverId ? 2 : 1)
  }
  const t = leaf?.data || cube?.data
  detail.value = t ? (t.type === 'entity'
    ? `${t.kind} · heat ${(+t.heat).toFixed(2)} — ${t.label}`
    : `${t.label} · ${cube.value} thoughts — scroll to enter`) : ''
}

function drill(dataNode) {
  if (dataNode.type !== 'kern' || !(dataNode.children || []).length) return
  const p = findPath(treeData, dataNode.id); if (p) { stack = p; hoverId = null; render() }
}
function out() { if (stack.length > 1) { stack.pop(); hoverId = null; render() } }
function goTo(id) { const i = stack.findIndex(n => n.id === id); if (i >= 0) { stack.length = i + 1; render() } }

function onWheel(ev) {
  ev.preventDefault()
  const now = Date.now(); if (now - wheelLock < 320) return; wheelLock = now
  if (ev.deltaY > 0) { const h = hit(ev); if (h?.cube?.data.type === 'kern') drill(h.cube.data) }
  else out()
}

async function load() {
  try {
    raw = await (await fetch('/graph')).json()
    kernsById = {}; entsByKern = new Map()
    for (const k of raw.kerns) kernsById[k.id] = k
    for (const e of raw.nodes) { if (!entsByKern.has(e.kern)) entsByKern.set(e.kern, []); entsByKern.get(e.kern).push(e) }
    const topo = raw.nodes.length + ':' + raw.kerns.length
    if (topo !== lastTopo) {
      lastTopo = topo; treeData = buildTree()
      const ids = stack.map(n => n.id); stack = [treeData]
      for (let i = 1; i < ids.length; i++) { const n = findById(treeData, ids[i]); if (n) stack.push(n); else break }
      render()
    }
    err.value = ''
  } catch (e) { err.value = String(e) }
}
function findById(n, id) { if (n.id === id) return n; for (const c of n.children || []) { const r = findById(c, id); if (r) return r } return null }

onMounted(() => {
  svg = d3.select(svgEl.value); g = svg.append('g')
  svgEl.value.addEventListener('wheel', onWheel, { passive: false })
  svgEl.value.addEventListener('mousemove', onMove)
  window.addEventListener('resize', () => render())
  load(); timer = setInterval(load, 5000)
})
onBeforeUnmount(() => { if (timer) clearInterval(timer); svgEl.value?.removeEventListener('wheel', onWheel); svgEl.value?.removeEventListener('mousemove', onMove) })
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
  <div class="legend">
    <span style="color:#98c379">■ claim</span><span style="color:#e5c07b">■ fact</span>
    <span style="color:#61afef">■ document</span><span style="color:#c678dd">■ question</span>
    <span class="dim">· hover to inspect · scroll ↓ into a cube · scroll ↑ out</span>
  </div>
  <div class="path">{{ detail }}</div>
  <svg ref="svgEl" class="tm"></svg>
</template>

<style>
html, body, #app { height: 100%; }
.tm { position: fixed; inset: 0; width: 100vw; height: 100vh; background: #06080b; display: block; }
.hud { position: fixed; top: 10px; left: 12px; z-index: 10; background: #11151aee; color: #cdd3da;
  padding: 8px 12px; border-radius: 8px; border: 1px solid #222a33;
  font: 13px system-ui, sans-serif; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; max-width: 92vw; }
.hud b { color: #7fd1ae; }
.crumbs a { color: #8fb6d8; cursor: pointer; }
.crumbs a.here { color: #e5c07b; font-weight: 600; }
.crumbs .sep { color: #4a5563; }
.stat { color: #9aa6b2; }
.err { color: #e06c75; }
.legend { position: fixed; top: 50px; left: 12px; z-index: 10; background: #11151acc; color: #9aa6b2;
  padding: 4px 10px; border-radius: 6px; font: 12px system-ui, sans-serif; display: flex; gap: 12px; }
.legend .dim { color: #5a6573; }
.path { position: fixed; bottom: 10px; left: 12px; right: 12px; z-index: 10; color: #cdd3da;
  font: 13px system-ui, sans-serif; background: #11151acc; padding: 6px 12px; border-radius: 6px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-height: 16px; }
</style>
