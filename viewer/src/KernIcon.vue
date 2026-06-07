<script setup>
const props = defineProps({
  n: String,
  size: { type: Number, default: 16 },
  sw: { type: Number, default: 1.6 },
})
</script>

<template>
  <!-- pen -->
  <svg v-if="n==='pen'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>
  <!-- search -->
  <svg v-else-if="n==='search'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
  <!-- inbox -->
  <svg v-else-if="n==='inbox'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M3 13h5l1.5 3h5L16 13h5"/><path d="M5 6h14l2 7v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-5Z"/></svg>
  <!-- branch -->
  <svg v-else-if="n==='branch'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="5" r="2.4"/><circle cx="18" cy="5" r="2.4"/><circle cx="12" cy="19" r="2.4"/><path d="M6 7.4v3a3 3 0 0 0 3 3h6a3 3 0 0 0 3-3v-3"/><line x1="12" y1="13.4" x2="12" y2="16.6"/></svg>
  <!-- thought -->
  <svg v-else-if="n==='thought'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.2l1.9 4.6 4.9.4-3.7 3.2 1.1 4.8L12 13.9 7.7 16.2l1.1-4.8L5.1 8.2l4.9-.4Z"/></svg>
  <!-- link -->
  <svg v-else-if="n==='link'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M9.5 14.5l5-5"/><path d="M11 7l1-1a3.4 3.4 0 0 1 4.9 4.8l-1 1"/><path d="M13 17l-1 1a3.4 3.4 0 0 1-4.9-4.8l1-1"/></svg>
  <!-- plus -->
  <svg v-else-if="n==='plus'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
  <!-- graph -->
  <svg v-else-if="n==='graph'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="7" r="2.3"/><circle cx="18" cy="7" r="2.3"/><circle cx="12" cy="17" r="2.3"/><line x1="8" y1="8" x2="10.5" y2="15.4"/><line x1="16" y1="8" x2="13.5" y2="15.4"/><line x1="8.2" y1="7" x2="15.8" y2="7"/></svg>
  <!-- flow -->
  <svg v-else-if="n==='flow'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h6a2 2 0 0 1 2 2v8a2 2 0 0 0 2 2h6"/><path d="M17 15l3 3-3 3"/><circle cx="4" cy="6" r="0.6"/></svg>
  <!-- relations -->
  <svg v-else-if="n==='relations'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="5" r="2.2"/><circle cx="5" cy="18" r="2.2"/><circle cx="19" cy="18" r="2.2"/><line x1="11" y1="7" x2="6" y2="16"/><line x1="13" y1="7" x2="18" y2="16"/><line x1="7" y1="18" x2="17" y2="18"/></svg>
  <!-- grid -->
  <svg v-else-if="n==='grid'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z"/></svg>
  <!-- arrowR -->
  <svg v-else-if="n==='arrowR'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="12" x2="19" y2="12"/><path d="M13 6l6 6-6 6"/></svg>
  <!-- send -->
  <svg v-else-if="n==='send'" :width="size" :height="size" viewBox="0 0 24 24" fill="currentColor" stroke="none" stroke-linecap="round" stroke-linejoin="round"><path d="M3.6 3.3 21 11.1a1 1 0 0 1 0 1.8L3.6 20.7a.7.7 0 0 1-1-.8l1.9-6.6a1 1 0 0 1 .9-.72l7.6-.58-7.6-.58a1 1 0 0 1-.9-.72L2.6 4.1a.7.7 0 0 1 1-.8Z"/></svg>
  <!-- check -->
  <svg v-else-if="n==='check'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>
  <!-- x -->
  <svg v-else-if="n==='x'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
  <!-- edit -->
  <svg v-else-if="n==='edit'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H5a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-6"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
  <!-- cut -->
  <svg v-else-if="n==='cut'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="2.6"/><circle cx="6" cy="18" r="2.6"/><line x1="8.3" y1="7.4" x2="20" y2="18"/><line x1="8.3" y1="16.6" x2="20" y2="6"/></svg>
  <!-- snow -->
  <svg v-else-if="n==='snow'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="3" x2="12" y2="21"/><line x1="3.6" y1="7.5" x2="20.4" y2="16.5"/><line x1="3.6" y1="16.5" x2="20.4" y2="7.5"/><path d="M9 4.5 12 6l3-1.5"/><path d="M9 19.5 12 18l3 1.5"/></svg>
  <!-- layers -->
  <svg v-else-if="n==='layers'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 3 8l9 5 9-5-9-5Z"/><path d="M3 13l9 5 9-5"/></svg>
  <!-- refresh -->
  <svg v-else-if="n==='refresh'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M3 11a9 9 0 0 1 15-5l3 3"/><path d="M21 4v6h-6"/><path d="M21 13a9 9 0 0 1-15 5l-3-3"/><path d="M3 20v-6h6"/></svg>
  <!-- sparkle -->
  <svg v-else-if="n==='sparkle'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v4M12 16v4M4 12h4M16 12h4"/><path d="M7 7l2 2M15 15l2 2M17 7l-2 2M9 15l-2 2"/></svg>
  <!-- panelR -->
  <svg v-else-if="n==='panelR'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="sw" stroke-linecap="round" stroke-linejoin="round"><path d="M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z"/><line x1="15" y1="5" x2="15" y2="19"/></svg>
</template>
