import vue from '@vitejs/plugin-vue'
import process from 'node:process'
import UnoCSS from 'unocss/vite'
import { defineConfig } from 'vite'

function stripCrossOrigin() {
  return {
    name: 'strip-crossorigin',
    enforce: 'post',
    transformIndexHtml(html) {
      return html.replace(/ crossorigin/g, '')
    },
  }
}

export default defineConfig({
  plugins: [
    vue(),
    UnoCSS(),
    stripCrossOrigin(),
  ],
  clearScreen: false,
  // Tauri应用需要使用相对路径
  base: './',
  server: {
    port: 5176,
    strictPort: true,
    host: '0.0.0.0',
    hmr: {
      port: 5178,
    },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: process.env.TAURI_PLATFORM === 'windows' ? 'chrome105' : 'safari13',
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    chunkSizeWarningLimit: 1500,
    rollupOptions: {
      output: {
        chunkFileNames: (chunkInfo) => {
          const name = (chunkInfo.name || 'chunk')
            .replace(/\.vue_vue_type_\w+.*$/, '')
            .replace(/\.vue$/, '')
          return `assets/${name}-[hash].js`
        },
        manualChunks: {
          vendor: ['vue', '@vueuse/core'],
          markdown: ['markdown-it', 'highlight.js'],
        },
      },
    },
  },
})
