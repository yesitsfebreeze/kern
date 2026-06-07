<script setup>
import { ref, nextTick } from 'vue'
import KernIcon from '../KernIcon.vue'

const props = defineProps({
  conv: Object,
  focusReply: String,
  stale: Object,
  resent: Object,
  busy: String,
  compose: String,
})
const emit = defineEmits(['setFocusReply', 'setCompose', 'send', 'resend'])

const taRef = ref(null)

function autosize(el) {
  if (!el) return
  el.style.height = 'auto'
  el.style.height = Math.min(el.scrollHeight, 200) + 'px'
}
function onKey(e) {
  if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); emit('send') }
}
function onInput(e) {
  emit('setCompose', e.target.value)
  autosize(e.target)
}

function paragraphs(m) {
  return Array.isArray(m.text) ? m.text : [m.text]
}
function nThoughts(m) { return (m.used || []).length }
function nReasons(m) { return (m.usedReasons || []).length }
</script>

<template>
  <div class="thread">
    <div class="thread-scroll">
      <div class="thread-inner">
        <div v-if="!conv.messages?.length" class="thread-empty" style="padding:24px 8px">
          <div class="te-t" style="font-size:16px">Write your first message</div>
          <div class="te-s">Fire it off like an email. kern replies with what it pulled from memory.</div>
        </div>

        <template v-for="m in conv.messages" :key="m.id">
          <!-- user message -->
          <div v-if="m.role==='you'" class="msg you">
            <div class="msg-by">
              <span class="avatar you">you</span>
              <span class="msg-who">You</span>
              <span class="grow"></span>
              <span class="msg-when">{{ m.when }}</span>
            </div>
            <div class="msg-card">
              <p v-for="(p, i) in paragraphs(m)" :key="i">{{ p }}</p>
            </div>
          </div>

          <!-- kern reply -->
          <div v-else class="msg kern">
            <div class="msg-by">
              <span class="avatar kern"><KernIcon n="layers" :size="13" /></span>
              <span class="msg-who">kern</span>
              <span class="grow"></span>
              <span v-if="resent[m.id]" class="status replied" style="color:var(--sage)">
                <span class="dot" style="background:var(--sage)"></span>re-sent
              </span>
              <span class="msg-when">{{ m.when }}</span>
            </div>
            <div class="msg-card">
              <p v-for="(p, i) in paragraphs(m)" :key="i">{{ p }}</p>
              <div class="lean">
                <button :class="['lean-chip', focusReply===m.id ? 'on' : '']" @click="emit('setFocusReply', m.id)">
                  <span class="ic"><KernIcon n="thought" :size="13" /></span>
                  leaned on <span class="lean-num">{{ nThoughts(m) }}</span> thought{{ nThoughts(m)!==1 ? 's' : '' }}
                </button>
                <button v-if="nReasons(m)>0" :class="['lean-chip', focusReply===m.id ? 'on' : '']" @click="emit('setFocusReply', m.id)">
                  <span class="ic"><KernIcon n="link" :size="13" /></span>
                  <span class="lean-num">{{ nReasons(m) }}</span> reason{{ nReasons(m)!==1 ? 's' : '' }}
                </button>
                <span class="grow"></span>
                <button class="lean-show" @click="emit('setFocusReply', m.id)">
                  <KernIcon n="layers" :size="13" /> show its work
                </button>
              </div>
              <div v-if="stale[m.id]" class="stale">
                <span class="st-ic"><KernIcon n="refresh" :size="16" /></span>
                <span class="st-tx"><b>Memory changed</b> since this answer. Send it back through to rethink.</span>
                <button class="resend" @click="emit('resend', m.id)">
                  <template v-if="busy===m.id">rethinking…</template>
                  <template v-else><KernIcon n="refresh" :size="14" /> Resend</template>
                </button>
              </div>
            </div>
          </div>
        </template>

        <!-- thinking indicator -->
        <div v-if="busy===conv.id" class="msg kern">
          <div class="msg-by">
            <span class="avatar kern"><KernIcon n="layers" :size="13" /></span>
            <span class="msg-who">kern</span>
          </div>
          <div class="msg-card" style="color:var(--ink-3);font-style:italic">pulling from memory…</div>
        </div>
      </div>
    </div>

    <div class="compose">
      <div class="compose-inner">
        <div class="compose-box">
          <textarea
            ref="taRef"
            :value="compose"
            rows="1"
            @input="onInput"
            @keydown="onKey"
            :placeholder="conv.messages?.length ? 'Reply, or push the thought further…' : 'Write your first message…'"
          ></textarea>
          <div class="compose-bar">
            <span class="compose-hint">grounded in memory · <span class="kbd">⌘</span><span class="kbd">↵</span> send</span>
            <span class="grow"></span>
            <button class="send-btn" :disabled="!compose.trim()" @click="emit('send')">
              <KernIcon n="send" :size="14" /> Send
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
