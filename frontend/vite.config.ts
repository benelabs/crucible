/// <reference types="vitest" />
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  build: {
    // Optimize production bundle
    minify: 'terser',
    terserOptions: {
      compress: {
        drop_console: true,
        drop_debugger: true
      }
    },
    // Code splitting configuration for large dependencies
    rollupOptions: {
      output: {
        manualChunks: {
          // Split vendor libraries into separate chunks
          'vendor-react': ['react', 'react-dom', 'react-i18next'],
          'vendor-charts': ['recharts'],
          'vendor-icons': ['lucide-react'],
          'vendor-i18n': ['i18next', 'i18next-browser-languagedetector']
        }
      }
    },
    // Chunk size warnings
    chunkSizeWarningLimit: 600,
    // Increase timeout for large builds
    commonjsOptions: {
      transformMixedEsm: true
    }
  },
  // Dynamic import optimization
  optimizeDeps: {
    include: [
      'react',
      'react-dom',
      'react-i18next',
      'recharts',
      'lucide-react',
      'i18next',
      'i18next-browser-languagedetector'
    ],
    exclude: ['@testing-library/react']
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: './src/setupTests.ts',
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      thresholds: {
        lines: 90,
        functions: 90,
        branches: 90,
        statements: 90
      }
    }
  }
})
