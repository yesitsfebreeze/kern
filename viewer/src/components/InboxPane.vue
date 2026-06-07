<script setup>
import KernIcon from '../KernIcon.vue'

const props = defineProps({
  activeThreads: Array,
  resolvedThreads: Array,
  branchesOf: Function,
  selId: String,
  count: Number,
})
const emit = defineEmits(['openThread', 'spawnConv'])

function lastSnippet(conv) {
  const last = conv.messages?.length ? conv.messages[conv.messages.length - 1] : null
  return last ? (Array.isArray(last.text) ? last.text.join(' ') : last.text) : 'No messages yet'
}
</script>

<template>
  <aside class="inbox">
    <div class="inbox-head">
      <span class="grow"></span>
      <span class="inbox-count">{{ count }} active</span>
    </div>
    <div class="inbox-scroll">
      <button class="compose-new" @click="emit('spawnConv')">
        <KernIcon n="pen" :size="14" /> New conversation
      </button>
      <div class="inbox-group">
        <span class="ey">Active</span>
        <template v-for="c in activeThreads" :key="c.id">
          <button :class="['thread-item', selId===c.id ? 'active' : '']" @click="emit('openThread', c.id)">
            <div class="ti-top">
              <span class="ti-subject">{{ c.subject }}</span>
              <span v-if="c.unread" class="ti-unread"></span>
              <span class="ti-time">{{ c.time }}</span>
            </div>
            <div class="ti-snippet">{{ lastSnippet(c) }}</div>
            <div class="ti-meta">
              <span :class="['status', c.status]"><span class="dot"></span>{{ c.status }}</span>
              <span v-if="branchesOf(c.id).length" class="ti-branchcount">
                <KernIcon n="branch" :size="12" /> {{ branchesOf(c.id).length }}
              </span>
            </div>
          </button>
          <button
            v-for="b in branchesOf(c.id)" :key="b.id"
            :class="['thread-item branch', selId===b.id ? 'active' : '']"
            @click="emit('openThread', b.id)"
          >
            <div class="ti-top">
              <span style="color:var(--ink-4);display:inline-flex"><KernIcon n="branch" :size="13" /></span>
              <span class="ti-subject">{{ b.subject }}</span>
              <span v-if="b.unread" class="ti-unread"></span>
              <span class="ti-time">{{ b.time }}</span>
            </div>
            <div class="ti-snippet">{{ lastSnippet(b) }}</div>
            <div class="ti-meta">
              <span :class="['status', b.status]"><span class="dot"></span>{{ b.status }}</span>
            </div>
          </button>
        </template>
        <div v-if="!activeThreads.length" class="ti-snippet" style="padding:6px 10px">nothing active — a calm inbox.</div>
      </div>
      <div v-if="resolvedThreads.length" class="inbox-group">
        <span class="ey">Settled</span>
        <template v-for="c in resolvedThreads" :key="c.id">
          <button :class="['thread-item', selId===c.id ? 'active' : '']" @click="emit('openThread', c.id)">
            <div class="ti-top">
              <span class="ti-subject">{{ c.subject }}</span>
              <span class="ti-time">{{ c.time }}</span>
            </div>
            <div class="ti-snippet">{{ lastSnippet(c) }}</div>
            <div class="ti-meta">
              <span :class="['status', c.status]"><span class="dot"></span>{{ c.status }}</span>
            </div>
          </button>
          <button
            v-for="b in branchesOf(c.id)" :key="b.id"
            :class="['thread-item branch', selId===b.id ? 'active' : '']"
            @click="emit('openThread', b.id)"
          >
            <div class="ti-top">
              <span style="color:var(--ink-4);display:inline-flex"><KernIcon n="branch" :size="13" /></span>
              <span class="ti-subject">{{ b.subject }}</span>
              <span class="ti-time">{{ b.time }}</span>
            </div>
            <div class="ti-snippet">{{ lastSnippet(b) }}</div>
          </button>
        </template>
      </div>
    </div>
  </aside>
</template>
