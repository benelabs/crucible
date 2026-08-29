/// <reference types="vitest" />
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import { visualizer } from 'rollup-plugin-visualizer'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    react(),
    // Bundle size visualization for CI/monitoring
    visualizer({
      filename: './dist/bundle-stats.html',
      open: false,
      gzipSize: true,
      brotliSize: true,
    })
  ],
  build: {
    // Optimize production bundle
    minify: 'terser',
    terserOptions: {
      compress: {
        drop_console: true,
        drop_debugger: true,
        pure_funcs: ['console.log', 'console.info'],
      },
      output: {
        comments: false,
      }
    },
    // Code splitting configuration for large dependencies
    rollupOptions: {
      output: {
        manualChunks(id) {
          // Split vendor libraries into separate chunks
          if (id.includes('node_modules')) {
            if (id.includes('react') || id.includes('react-dom')) {
              return 'vendor-react'
            } else if (id.includes('recharts')) {
              return 'vendor-charts'
            } else if (id.includes('lucide-react')) {
              return 'vendor-icons'
            } else if (id.includes('i18next')) {
              return 'vendor-i18n'
            } else {
              return 'vendor-other'
            }
          }
        },
        // Minimize entry point
        entryFileNames: 'js/[name]-[hash].js',
        chunkFileNames: 'js/[name]-[hash].js',
        assetFileNames: ({ name }) => {
          if (name.endsWith('.css')) return 'css/[name]-[hash][extname]'
          if (name.match(/\.(png|jpg|jpeg|gif|svg)$/)) return 'images/[name]-[hash][extname]'
          return 'assets/[name]-[hash][extname]'
        }
      }
    },
    // Chunk size warnings - enforced max 150KB initial, 300KB total
    chunkSizeWarningLimit: 300,
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
    // Increase timeout for large builds
    commonjsOptions: {
      transformMixedEsm: true,
      strictRequires: true
    },
    // CSS code splitting
    cssCodeSplit: true,
    // Source maps for production debugging
    sourcemap: 'hidden',
    // Report compressed sizes
    reportCompressedSize: true,
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
      reporter: ['text', 'json', 'html', 'lcov'],
      thresholds: {
        lines: 90,
        functions: 90,
        branches: 90,
        statements: 90
      },
      exclude: [
        'node_modules/',
        'src/setupTests.ts',
      ]
    }
  }
})
