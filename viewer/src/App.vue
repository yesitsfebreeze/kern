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
let kernsById = {}, entsByKern = new Map(), meanHeat = {}
let treeData = null
let stack = []
let layoutRoot = null
let lastTopo = ''
let hoverId = null
let wheelLock = 0

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
function meanHeatOf(node) {
  if (node.type === 'entity') return +node.heat || 0
  let s = 0, n = 0
  const walk = (x) => { if (x.type === 'entity') { s += +x.heat || 0; n++ } else for (const c of x.children || []) walk(c) }
  walk(node)
  return n ? s / n : 0
}
function findPath(node, id, acc = []) {
  acc.push(node); if (node.id === id) return acc.slice()
  for (const c of node.children || []) { const r = findPath(c, id, acc); if (r) return r }
  acc.pop(); return null
}
function findById(n, id) { if (n.id === id) return n; for (const c of n.children || []) { const r = findById(c, id); if (r) return r } return null }

// warm intensity ramp — the look from the references.
const heatMax = () => Math.max(0.5, d3.max(raw.nodes, n => +n.heat || 0) || 1)
function ramp(h) { return d3.interpolateInferno(0.2 + 0.72 * Math.sqrt(Math.min(1, (h || 0) / heatMax()))) }

const sideOf = () => Math.max(140, Math.min(svgEl.value.clientWidth, svgEl.value.clientHeight) - 80)
let ox = 0, oy = 0
let svg, g

// count of thoughts under a data node (for tile size)
function d3Count(d) {
  if (d.type === 'entity') return 1
  let n = 0; const walk = x => { if (x.type === 'entity') n++; else for (const c of x.children || []) walk(c) }; walk(d); return n || 1
}
function layout() {
  const cur = stack[stack.length - 1]
  const side = sideOf()
  ox = (svgEl.value.clientWidth - side) / 2
  oy = (svgEl.value.clientHeight - side) / 2
  // ONE level only: the current node's DIRECT children, squarified, sized by
  // how many thoughts each contains (biggest topic = most area).
  const kids = (cur.children || []).map(c => ({ ref: c, value: d3Count(c) }))
  const r = d3.hierarchy({ id: '_root', children: kids })
    .sum(d => d.value || 0)
    .sort((a, b) => (b.value || 0) - (a.value || 0))
  d3.treemap().tile(d3.treemapSquarify.ratio(1)).size([side, side]).round(true).padding(7)(r)
  return r
}

function render(dir) {
  crumbs.value = stack.map((n, i) => ({ id: n.id, label: i === 0 ? 'root' : n.label }))
  layoutRoot = layout()
  const tiles = layoutRoot.children || []
  g.attr('transform', `translate(${ox},${oy})`)
  const W = d => Math.max(0, d.x1 - d.x0), H = d => Math.max(0, d.y1 - d.y0)
  const cx = sideOf() / 2

  const sel = g.selectAll('g.tile').data(tiles, d => d.data.ref.id)
  sel.exit().remove()
  const ent = sel.enter().append('g').attr('class', 'tile')
  ent.append('rect').attr('rx', 7).attr('ry', 7)
  ent.append('text').attr('class', 'lab')
  ent.append('text').attr('class', 'cnt')
  const all = ent.merge(sel)

  all.style('cursor', d => d.data.ref.type === 'kern' ? 'pointer' : 'default')
    .on('mouseenter', (ev, d) => setHover(d.data.ref.id))
    .on('click', (ev, d) => { if (d.data.ref.type === 'kern') drill(d.data.ref) })

  all.select('rect')
    .attr('fill', d => ramp(d.data.ref.type === 'entity' ? d.data.ref.heat : meanHeat[d.data.ref.id]))
    .attr('stroke', d => d.data.ref.id === hoverId ? '#fff' : 'rgba(255,255,255,0.08)')
    .attr('stroke-width', d => d.data.ref.id === hoverId ? 2 : 1)

  all.select('text.lab')
    .attr('fill', d => '#f4ece2')
    .attr('font-size', d => Math.min(15, 9 + W(d) / 40) + 'px')
    .attr('font-weight', 600)
    .attr('x', d => d.x0 + 8).attr('y', d => d.y0 + 18)
    .style('pointer-events', 'none')
    .style('paint-order', 'stroke').style('stroke', 'rgba(0,0,0,0.55)').style('stroke-width', '2.5px')
    .text(d => W(d) > 40 && H(d) > 16 ? clip(d.data.ref.label, W(d) - 12) : '')

  all.select('text.cnt')
    .attr('fill', 'rgba(255,255,255,0.7)').attr('font-size', '11px')
    .attr('x', d => d.x0 + 8).attr('y', d => d.y0 + 33)
    .style('pointer-events', 'none')
    .style('paint-order', 'stroke').style('stroke', 'rgba(0,0,0,0.45)').style('stroke-width', '2px')
    .text(d => d.data.ref.type === 'kern' && W(d) > 50 && H(d) > 34 ? d.value + ' thoughts' : '')

  // place + animate (scroll-in = grow from center; scroll-out = shrink in).
  const place = (s) => s.attr('transform', 'translate(0,0) scale(1)')
  all.select('rect').attr('x', d => d.x0).attr('y', d => d.y0).attr('width', W).attr('height', H)
  if (dir) {
    const k = dir === 'in' ? 0.7 : 1.25
    all.attr('transform', d => `translate(${cx - (cx) * k},${cx - cx * k}) scale(${k})`).style('opacity', 0)
      .transition().duration(360).ease(d3.easeCubicOut)
      .attr('transform', 'translate(0,0) scale(1)').style('opacity', 1)
  } else {
    all.attr('transform', 'translate(0,0) scale(1)').style('opacity', 1)
  }

  stats.value = `${raw.nodes.length} thoughts · ${raw.kerns.length} spheres · here: ${layoutRoot.value}`
}

function clip(s, px) { const n = Math.floor(px / 6.4); return n >= s.length ? s : (n > 1 ? s.slice(0, n - 1) + '…' : '') }

function setHover(id) {
  if (id === hoverId) return
  hoverId = id
  g.selectAll('g.tile rect').attr('stroke', d => d.data.ref.id === hoverId ? '#fff' : 'rgba(255,255,255,0.08)')
    .attr('stroke-width', d => d.data.ref.id === hoverId ? 2 : 1)
  const d = (layoutRoot.children || []).find(c => c.data.ref.id === id)?.data.ref
  detail.value = d ? (d.type === 'entity'
    ? `${d.kind} · heat ${(+d.heat).toFixed(2)} · conf ${(+d.conf).toFixed(2)} — ${d.label}`
    : `${d.label} · ${d3Count(d)} thoughts — scroll to enter`) : ''
}

function drill(dataNode) {
  if (dataNode.type !== 'kern' || !(dataNode.children || []).length) return
  const p = findPath(treeData, dataNode.id); if (p) { stack = p; hoverId = null; render('in') }
}
function out() { if (stack.length > 1) { stack.pop(); hoverId = null; render('out') } }
function goTo(id) { const i = stack.findIndex(n => n.id === id); if (i >= 0) { stack.length = i + 1; render('out') } }

function onWheel(ev) {
  ev.preventDefault()
  const now = Date.now(); if (now - wheelLock < 360) return; wheelLock = now
  if (ev.deltaY > 0) { const d = (layoutRoot.children || []).find(c => c.data.ref.id === hoverId)?.data.ref; if (d?.type === 'kern') drill(d) }
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
      meanHeat = {}; const reg = x => { if (x.type === 'kern') { meanHeat[x.id] = meanHeatOf(x); for (const c of x.children || []) reg(c) } }; reg(treeData)
      const ids = stack.map(n => n.id); stack = [treeData]
      for (let i = 1; i < ids.length; i++) { const n = findById(treeData, ids[i]); if (n) stack.push(n); else break }
      render(null)
    }
    err.value = ''
  } catch (e) { err.value = String(e) }
}

onMounted(() => {
  svg = d3.select(svgEl.value); g = svg.append('g')
  svgEl.value.addEventListener('wheel', onWheel, { passive: false })
  window.addEventListener('resize', () => render(null))
  load(); timer = setInterval(load, 5000)
})
onBeforeUnmount(() => { if (timer) clearInterval(timer); svgEl.value?.removeEventListener('wheel', onWheel) })
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
  <div class="path">{{ detail || 'hover a cube · scroll ↓ to dive into it · scroll ↑ to go back' }}</div>
  <svg ref="svgEl" class="tm"></svg>
</template>

<style>
html, body, #app { height: 100%; }
.tm { position: fixed; inset: 0; width: 100vw; height: 100vh; background: #07090d; display: block; }
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
