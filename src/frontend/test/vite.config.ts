/// <reference types="node" />
import vue from '@vitejs/plugin-vue'
import { resolve } from 'node:path'
import UnoCSS from 'unocss/vite'
import { defineConfig } from 'vite'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    UnoCSS({
      configFile: resolve(__dirname, '../../../uno.config.ts'),
    }),
  ],
  resolve: {
    alias: {
      '@': resolve(__dirname, '../'),
    },
  },
  root: __dirname,
  server: {
    port: 5174, // 使用不同的端口避免冲突
    open: true,
  },
  build: {
    outDir: resolve(__dirname, '../../../dist-test'),
    emptyOutDir: true,
  },
})
