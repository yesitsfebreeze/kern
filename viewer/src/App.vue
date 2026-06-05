<script setup>
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import * as d3 from 'd3'

const raw = ref({ nodes: [], links: [], kerns: [] })
const err = ref('')
const hovered = ref(null)
let timer = null

const kernsById = computed(() => {
  const m = {}; for (const k of raw.value.kerns) m[k.id] = k; return m
})
const entsByKern = computed(() => {
  const m = new Map()
  for (const e of raw.value.nodes) { if (!m.has(e.kern)) m.set(e.kern, []); m.get(e.kern).push(e) }
  for (const arr of m.values()) arr.sort((a, b) => (b.heat || 0) - (a.heat || 0))
  return m
})
const maxHeat = computed(() => Math.max(0.001, d3.max(raw.value.nodes, n => +n.heat || 0) || 1))

// DFS the sphere tree → ordered, indented groups; only kerns with thoughts show
// a cell grid, but every named level keeps the hierarchy readable.
const groups = computed(() => {
  const out = []
  const root = raw.value.kerns.find(k => !k.parent) || raw.value.kerns[0]
  if (!root) return out
  const walk = (id, depth, seen) => {
    const k = kernsById.value[id]
    if (!k || seen.has(id)) return
    seen.add(id)
    const ents = entsByKern.value.get(id) || []
    out.push({ id, label: k.named ? k.label : '(unnamed)', named: k.named, depth, count: ents.length, total: k.count, ents })
    for (const c of k.children || []) walk(c, depth + 1, seen)
  }
  walk(root.id, 0, new Set())
  return out
})

const total = computed(() => raw.value.nodes.length)
const topColor = (depth) => d3.lab(70 - depth * 6, 6, 8).formatHex()
function heatColor(h) {
  // warm intensity ramp (like the references); low heat = dim ember, high = bright.
  return d3.interpolateInferno(0.15 + 0.8 * Math.sqrt(Math.min(1, (h || 0) / maxHeat.value)))
}
const kindMark = { Fact: '◆', Document: '■', Question: '▲', Claim: '●' }

async function load() {
  try { raw.value = await (await fetch('/graph')).json(); err.value = '' }
  catch (e) { err.value = String(e) }
}
onMounted(() => { load(); timer = setInterval(load, 5000) })
onBeforeUnmount(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="wrap">
    <header>
      <span class="brand">kern</span>
      <span class="sub">{{ total }} thoughts · {{ raw.kerns.length }} spheres</span>
      <span class="legend">heat&nbsp;<i class="ramp"></i>&nbsp;hot</span>
      <span v-if="err" class="err">{{ err }}</span>
    </header>

    <main>
      <section v-for="g in groups" :key="g.id" class="grp" :style="{ paddingLeft: 14 + g.depth * 18 + 'px' }">
        <div class="hdr">
          <i class="dot" :style="{ background: topColor(g.depth) }"></i>
          <span class="name" :class="{ unnamed: !g.named }">{{ g.label }}</span>
          <span class="cnt">{{ g.count }}<span v-if="g.total > g.count" class="sub2"> / {{ g.total }} below</span></span>
        </div>
        <div class="cells">
          <span v-for="e in g.ents" :key="e.id" class="cell"
            :style="{ background: heatColor(e.heat) }"
            :title="`${e.kind} · heat ${(+e.heat).toFixed(2)} · conf ${(+e.conf).toFixed(2)}\n${e.label}`"
            @mouseenter="hovered = e" />
        </div>
      </section>
    </main>

    <footer>
      <template v-if="hovered">
        <span class="mk">{{ kindMark[hovered.kind] || '·' }}</span>
        <span class="hk">{{ hovered.kind }}</span>
        <span class="ht">{{ hovered.label }}</span>
      </template>
      <span v-else class="dim">hover a cell · color = heat · sections = topic spheres</span>
    </footer>
  </div>
</template>

<style>
* { box-sizing: border-box; }
html, body, #app { height: 100%; margin: 0; }
.wrap { height: 100vh; display: flex; flex-direction: column; background: #0a0c10; color: #c8cfd8;
  font: 13px/1.4 ui-sans-serif, system-ui, sans-serif; }

header { display: flex; align-items: center; gap: 16px; padding: 12px 18px; border-bottom: 1px solid #1a2028; }
.brand { color: #7fd1ae; font-weight: 700; letter-spacing: .5px; }
.sub { color: #6b7682; }
.legend { margin-left: auto; color: #6b7682; display: flex; align-items: center; }
.ramp { display: inline-block; width: 90px; height: 9px; border-radius: 5px;
  background: linear-gradient(90deg, #2a1a12, #d2602a, #f6d29a); }
.err { color: #e06c75; }

main { flex: 1; overflow-y: auto; padding: 14px 18px 40px; }
.grp { margin-bottom: 16px; }
.hdr { display: flex; align-items: baseline; gap: 8px; margin-bottom: 6px; }
.dot { width: 8px; height: 8px; border-radius: 50%; flex: none; align-self: center; }
.name { font-weight: 600; color: #dbe2ea; }
.name.unnamed { color: #6b7682; font-weight: 400; font-style: italic; }
.cnt { color: #5d6772; font-size: 12px; }
.sub2 { color: #444d57; }

.cells { display: flex; flex-wrap: wrap; gap: 3px; }
.cell { width: 13px; height: 13px; border-radius: 2px; cursor: default;
  transition: transform .08s; }
.cell:hover { transform: scale(1.5); outline: 1px solid #cfe9df; }

footer { padding: 8px 18px; border-top: 1px solid #1a2028; display: flex; gap: 10px; align-items: center;
  white-space: nowrap; overflow: hidden; }
.mk { color: #e5c07b; }
.hk { color: #7fd1ae; font-weight: 600; }
.ht { color: #aeb7c1; overflow: hidden; text-overflow: ellipsis; }
.dim { color: #555f6a; }
</style>
