<script setup>
const ACCENTS = ['#CF5320', '#C2410C', '#B8455C', '#4F7A8C', '#5E7D5A', '#6D5B8A']

const props = defineProps({ theme: String, density: String, accent: String })
const emit = defineEmits(['setTheme', 'setDensity', 'setAccent'])

const keys = [
  ['address a tile', ['1–3', 'then', '1/2']],
  ['focus column', ['alt', '←', '→']],
  ['focus row', ['alt', '↑', '↓']],
  ['add tab', ['alt', 't']],
  ['split column', ['alt', 's']],
  ['new conversation', ['alt', 'n']],
  ['before / after', ['alt', 'p']],
  ['settings', ['alt', ',']],
  ['close window', ['alt', 'w']],
]
</script>

<template>
  <div class="settings">
    <div class="set-group">
      <span class="ey">Appearance</span>
      <div class="set-row">
        <div class="set-lab"><b>Theme</b><span>dark for night work; light is calmer</span></div>
        <div class="seg">
          <button :class="theme==='light' ? 'on' : ''" @click="emit('setTheme', 'light')">light</button>
          <button :class="theme==='dark' ? 'on' : ''" @click="emit('setTheme', 'dark')">dark</button>
        </div>
      </div>
      <div class="set-row">
        <div class="set-lab"><b>Density</b><span>room to breathe, or more on screen</span></div>
        <div class="seg">
          <button :class="density==='comfortable' ? 'on' : ''" @click="emit('setDensity', 'comfortable')">comfortable</button>
          <button :class="density==='compact' ? 'on' : ''" @click="emit('setDensity', 'compact')">compact</button>
        </div>
      </div>
      <div class="set-row">
        <div class="set-lab"><b>Accent</b><span>the one warm signal</span></div>
        <div class="swatches">
          <button v-for="c in ACCENTS" :key="c"
            :class="['swatch', accent===c ? 'on' : '']"
            :style="{ background: c, '--sw': c }"
            @click="emit('setAccent', c)"
          ></button>
        </div>
      </div>
    </div>
    <div class="set-group">
      <span class="ey">Workspace</span>
      <div class="keys">
        <div v-for="(k, i) in keys" :key="i" class="keyrow">
          <span class="kdesc">{{ k[0] }}</span>
          <span class="kcombo">
            <span v-for="(kk, j) in k[1]" :key="j" class="kbd">{{ kk }}</span>
          </span>
        </div>
      </div>
    </div>
    <div class="set-group">
      <span class="ey">About</span>
      <div class="set-row">
        <div class="set-lab">
          <span>kern is a spatial memory you author and arrange. Up to three columns, each split once — six tiles, never more. The layout remembers not just what you arranged, but why.</span>
        </div>
      </div>
    </div>
  </div>
</template>
