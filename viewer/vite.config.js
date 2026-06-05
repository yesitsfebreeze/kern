import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Dev server proxies the kern data API so the app fetches it same-origin —
// no CORS, and `npm run dev` gives instant HMR while kern keeps running.
// Point at a different daemon with KERN_URL=http://host:port npm run dev.
const KERN = process.env.KERN_URL || 'http://127.0.0.1:7700'

export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: {
      '/graph': { target: KERN, changeOrigin: true },
      // POST + SSE stream for the oracle. http-proxy streams chunked responses
      // through unbuffered, so token events arrive live.
      '/ask': { target: KERN, changeOrigin: true },
    },
  },
})
